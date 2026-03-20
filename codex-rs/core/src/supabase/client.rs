use reqwest::Client;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;

/// Low-level HTTP client for Supabase REST and Auth endpoints.
///
/// Holds the project URL, anon key, and a shared `reqwest::Client`.
#[derive(Debug, Clone)]
pub struct SupabaseClient {
    base_url: String,
    anon_key: String,
    client: Client,
}

impl SupabaseClient {
    /// Create a new `SupabaseClient`.
    ///
    /// `base_url` is the Supabase project URL (e.g.
    /// `https://xyzproject.supabase.co`). Trailing slashes are stripped.
    pub fn new(base_url: impl Into<String>, anon_key: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let anon_key = anon_key.into();
        let client = Client::new();
        Self {
            base_url,
            anon_key,
            client,
        }
    }

    /// Build the full URL for a Supabase Auth endpoint.
    pub fn auth_url(&self, path: &str) -> String {
        format!("{}/auth/v1/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// Build the full URL for a Supabase REST (PostgREST) endpoint.
    pub fn rest_url(&self, path: &str) -> String {
        format!("{}/rest/v1/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// Default headers required for Supabase API calls (apikey + content-type).
    pub fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(val) = HeaderValue::from_str(&self.anon_key) {
            headers.insert("apikey", val.clone());
            headers.insert("Authorization", {
                let bearer = format!("Bearer {}", self.anon_key);
                HeaderValue::from_str(&bearer).unwrap_or(val)
            });
        }
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers
    }

    /// Headers with a specific bearer token (for authenticated requests).
    pub fn auth_headers(&self, access_token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(val) = HeaderValue::from_str(&self.anon_key) {
            headers.insert("apikey", val);
        }
        if let Ok(val) = HeaderValue::from_str(&format!("Bearer {access_token}")) {
            headers.insert("Authorization", val);
        }
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers
    }

    /// Access the underlying `reqwest::Client`.
    pub fn http(&self) -> &Client {
        &self.client
    }

    /// The base URL for this Supabase project.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The anon key for this Supabase project.
    pub fn anon_key(&self) -> &str {
        &self.anon_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_strips_trailing_slash() {
        let client = SupabaseClient::new("https://example.supabase.co/", "anon-key-123");
        assert_eq!(client.base_url(), "https://example.supabase.co");
    }

    #[test]
    fn auth_url_building() {
        let client = SupabaseClient::new("https://example.supabase.co", "key");
        assert_eq!(
            client.auth_url("token?grant_type=refresh_token"),
            "https://example.supabase.co/auth/v1/token?grant_type=refresh_token"
        );
        assert_eq!(
            client.auth_url("/otp"),
            "https://example.supabase.co/auth/v1/otp"
        );
    }

    #[test]
    fn rest_url_building() {
        let client = SupabaseClient::new("https://example.supabase.co", "key");
        assert_eq!(
            client.rest_url("profiles"),
            "https://example.supabase.co/rest/v1/profiles"
        );
        assert_eq!(
            client.rest_url("/devices?user_id=eq.abc"),
            "https://example.supabase.co/rest/v1/devices?user_id=eq.abc"
        );
    }

    #[test]
    fn default_headers_contain_apikey() {
        let client = SupabaseClient::new("https://example.supabase.co", "test-anon-key");
        let headers = client.default_headers();
        assert_eq!(headers.get("apikey").unwrap(), "test-anon-key");
        assert_eq!(headers.get("Content-Type").unwrap(), "application/json");
        assert!(
            headers
                .get("Authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("Bearer ")
        );
    }

    #[test]
    fn auth_headers_use_access_token() {
        let client = SupabaseClient::new("https://example.supabase.co", "anon-key");
        let headers = client.auth_headers("my-access-token");
        assert_eq!(headers.get("apikey").unwrap(), "anon-key");
        assert_eq!(
            headers.get("Authorization").unwrap(),
            "Bearer my-access-token"
        );
    }

    #[test]
    fn anon_key_accessor() {
        let client = SupabaseClient::new("https://example.supabase.co", "secret-anon");
        assert_eq!(client.anon_key(), "secret-anon");
    }
}
