use async_trait::async_trait;

use crate::reference_resolution::{
    AuthorizedCandidateFetch, AuthorizedDirectFetch, AuthorizedSearch, DynamicCandidateSealer,
    Provider, QueryLocale,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryCandidate {
    pub(crate) ordinal: u32,
    pub(crate) url: String,
    pub(crate) source_identity: String,
}

#[derive(Debug)]
pub(crate) struct AuthorizedDiscoveryResult {
    pub(crate) candidates: Vec<crate::reference_resolution::ProviderOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorizedFetchResult {
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthorizedAdapterError {
    Timeout,
    ConnectionReset,
    RateLimited,
    Http5xx(u16),
    Http4xx(u16),
    InvalidResponse,
    Transport,
    Candidate(crate::reference_resolution::AuthorizationDenial),
}

#[async_trait]
pub(crate) trait AuthorizedWebTransport: Send + Sync {
    async fn search(
        &self,
        provider: Provider,
        query: &str,
        locale: QueryLocale,
    ) -> Result<Vec<DiscoveryCandidate>, AuthorizedAdapterError>;

    async fn fetch(
        &self,
        provider: Provider,
        url: &str,
    ) -> Result<AuthorizedFetchResult, AuthorizedAdapterError>;
}

pub(crate) struct AuthorizedWebAdapter<T> {
    transport: T,
}

impl<T> AuthorizedWebAdapter<T> {
    pub(crate) fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: AuthorizedWebTransport> AuthorizedWebAdapter<T> {
    pub(crate) async fn search(
        &self,
        operation: AuthorizedSearch,
    ) -> Result<AuthorizedDiscoveryResult, AuthorizedAdapterError> {
        let provider = operation.provider();
        let locale = operation.locale();
        let query = operation.with_query(str::to_owned);
        let candidates = self.transport.search(provider, &query, locale).await?;
        let sealer: DynamicCandidateSealer = operation.candidate_sealer();
        let mut sealed = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        for candidate in candidates {
            let normalized =
                crate::reference_resolution::normalize_public_url_for_adapter(&candidate.url)
                    .map_err(|_| {
                        AuthorizedAdapterError::Candidate(
                            crate::reference_resolution::AuthorizationDenial::UnsafeValue,
                        )
                    })?;
            if !seen_urls.insert(normalized) {
                continue;
            }
            let operation = sealer
                .seal_candidate(
                    candidate.ordinal,
                    &candidate.url,
                    &candidate.source_identity,
                    sealed.len() as u8,
                )
                .await
                .map_err(AuthorizedAdapterError::Candidate)?;
            // The candidate operation remains sealed and is returned through
            // the next adapter boundary. Discovery itself never fetches it.
            sealed.push(operation);
        }
        Ok(AuthorizedDiscoveryResult { candidates: sealed })
    }

    pub(crate) async fn fetch_candidate(
        &self,
        operation: AuthorizedCandidateFetch,
    ) -> Result<AuthorizedFetchResult, AuthorizedAdapterError> {
        let provider = operation.provider();
        let url = operation.with_url(str::to_owned);
        self.transport.fetch(provider, &url).await
    }

    pub(crate) async fn fetch_direct(
        &self,
        operation: AuthorizedDirectFetch,
    ) -> Result<AuthorizedFetchResult, AuthorizedAdapterError> {
        let url = operation.with_url(str::to_owned);
        self.transport.fetch(Provider::Direct, &url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct CountingTransport {
        fetches: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AuthorizedWebTransport for CountingTransport {
        async fn search(
            &self,
            _provider: Provider,
            _query: &str,
            _locale: QueryLocale,
        ) -> Result<Vec<DiscoveryCandidate>, AuthorizedAdapterError> {
            Ok(Vec::new())
        }

        async fn fetch(
            &self,
            _provider: Provider,
            _url: &str,
        ) -> Result<AuthorizedFetchResult, AuthorizedAdapterError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            Ok(AuthorizedFetchResult {
                body: b"synthetic".to_vec(),
            })
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn direct_adapter_uses_one_typed_authorized_call() {
        let fetches = Arc::new(AtomicUsize::new(0));
        let adapter = AuthorizedWebAdapter::new(CountingTransport {
            fetches: Arc::clone(&fetches),
        });
        let result = adapter
            .fetch_direct(crate::reference_resolution::test_authorized_direct_fetch(
                "https://example.test/public",
            ))
            .await
            .expect("synthetic authorized direct fetch");
        assert_eq!(result.body, b"synthetic");
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }
}
