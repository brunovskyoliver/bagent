import Foundation
import Security

/// Simple Keychain wrapper for storing string values under named keys.
///
/// Keys are stored in the default keychain for the current user under the
/// `kSecClassGenericPassword` class. Each key gets its own entry; existing
/// entries are overwritten by `save(key:value:)`.
enum KeychainStore {

    // MARK: - Save

    /// Persist `value` under `key`. Overwrites any existing entry.
    @discardableResult
    static func save(key: String, value: String) -> Bool {
        guard let data = value.data(using: .utf8) else { return false }

        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: Bundle.main.bundleIdentifier ?? "com.bagent.app",
            kSecAttrAccount as String: key,
            kSecValueData as String:   data,
        ]

        // Delete existing entry first (delete+add is simpler than update).
        SecItemDelete(query as CFDictionary)

        let status = SecItemAdd(query as CFDictionary, nil)
        if status != errSecSuccess {
            print("[KeychainStore] save failed for key=\(key) status=\(status)")
        }
        return status == errSecSuccess
    }

    // MARK: - Load

    /// Load the string stored under `key`, or `nil` if absent.
    static func load(key: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: Bundle.main.bundleIdentifier ?? "com.bagent.app",
            kSecAttrAccount as String: key,
            kSecReturnData as String:  true,
            kSecMatchLimit as String:  kSecMatchLimitOne,
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess,
              let data = result as? Data,
              let value = String(data: data, encoding: .utf8)
        else { return nil }
        return value
    }

    // MARK: - Delete

    @discardableResult
    static func delete(key: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: Bundle.main.bundleIdentifier ?? "com.bagent.app",
            kSecAttrAccount as String: key,
        ]
        let status = SecItemDelete(query as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }
}

// MARK: - Odoo-specific keys

extension KeychainStore {
    static let odooURLKey    = "bagent.odoo.url"
    static let odooDBKey     = "bagent.odoo.db"
    static let odooUserKey   = "bagent.odoo.user"
    static let odooAPIKeyKey = "bagent.odoo.apikey"

    static func saveOdoo(url: String, db: String, user: String, apiKey: String) {
        save(key: odooURLKey,    value: url)
        save(key: odooDBKey,     value: db)
        save(key: odooUserKey,   value: user)
        save(key: odooAPIKeyKey, value: apiKey)
    }

    static func loadOdoo() -> (url: String, db: String, user: String, apiKey: String)? {
        guard
            let url    = load(key: odooURLKey),
            let db     = load(key: odooDBKey),
            let user   = load(key: odooUserKey),
            let apiKey = load(key: odooAPIKeyKey)
        else { return nil }
        return (url, db, user, apiKey)
    }

    static func deleteOdoo() {
        delete(key: odooURLKey)
        delete(key: odooDBKey)
        delete(key: odooUserKey)
        delete(key: odooAPIKeyKey)
    }
}
