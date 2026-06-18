import Foundation
import Security

/// Simple Keychain wrapper for storing string values under named keys.
///
/// Keys are stored in the default keychain for the current user under the
/// `kSecClassGenericPassword` class. Each key gets its own entry; existing
/// entries are overwritten by `save(key:value:)`.
@MainActor
enum KeychainStore {
    private static let service = "sk.bagent.app"
    private static var cache: [String: String] = [:]

    // MARK: - Save

    /// Persist `value` under `key`. Overwrites any existing entry.
    @discardableResult
    static func save(key: String, value: String) -> Bool {
        guard let data = value.data(using: .utf8) else { return false }

        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
        let updateAttrs: [String: Any] = [
            kSecValueData as String: data,
        ]

        let updateStatus = SecItemUpdate(query as CFDictionary, updateAttrs as CFDictionary)
        let status: OSStatus
        if updateStatus == errSecItemNotFound {
            var addQuery = query
            updateAttrs.forEach { addQuery[$0.key] = $0.value }
            addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            status = SecItemAdd(addQuery as CFDictionary, nil)
        } else {
            status = updateStatus
        }
        if status != errSecSuccess {
            print("[KeychainStore] save failed for key=\(key) status=\(status)")
        }
        if status == errSecSuccess {
            cache[key] = value
        }
        return status == errSecSuccess
    }

    // MARK: - Load

    /// Load the string stored under `key`, or `nil` if absent.
    static func load(key: String) -> String? {
        if let value = cache[key] { return value }
        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: service,
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
        cache[key] = value
        return value
    }

    // MARK: - Delete

    @discardableResult
    static func delete(key: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
        let status = SecItemDelete(query as CFDictionary)
        cache.removeValue(forKey: key)
        return status == errSecSuccess || status == errSecItemNotFound
    }
}

// MARK: - Odoo-specific keys

extension KeychainStore {
    static let odooURLKey    = "bagent.odoo.url"
    static let odooDBKey     = "bagent.odoo.db"
    static let odooUserKey   = "bagent.odoo.user"
    static let odooAPIKeyKey = "bagent.odoo.apikey"
    static let odooCredentialsKey = "bagent.odoo.credentials.v2"

    private struct OdooCredentials: Codable {
        let url: String
        let db: String
        let user: String
        let apiKey: String
    }

    static func saveOdoo(url: String, db: String, user: String, apiKey: String) {
        let creds = OdooCredentials(url: url, db: db, user: user, apiKey: apiKey)
        if let data = try? JSONEncoder().encode(creds),
           let json = String(data: data, encoding: .utf8) {
            save(key: odooCredentialsKey, value: json)
        } else {
            save(key: odooAPIKeyKey, value: apiKey)
        }
    }

    static func loadOdoo() -> (url: String, db: String, user: String, apiKey: String)? {
        if let json = load(key: odooCredentialsKey),
           let data = json.data(using: .utf8),
           let creds = try? JSONDecoder().decode(OdooCredentials.self, from: data) {
            return (creds.url, creds.db, creds.user, creds.apiKey)
        }

        let defaults = UserDefaults.standard
        let url = defaults.string(forKey: odooURLKey) ?? ""
        let db = defaults.string(forKey: odooDBKey) ?? ""
        let user = defaults.string(forKey: odooUserKey) ?? ""
        guard !url.isEmpty, !db.isEmpty, !user.isEmpty,
              let apiKey = load(key: odooAPIKeyKey) else { return nil }
        saveOdoo(url: url, db: db, user: user, apiKey: apiKey)
        return (url, db, user, apiKey)
    }

    static func loadOdooAPIKey() -> String? {
        if let creds = loadOdoo() { return creds.apiKey }
        return load(key: odooAPIKeyKey)
    }

    static func deleteOdoo() {
        delete(key: odooCredentialsKey)
        delete(key: odooAPIKeyKey)
    }
}
