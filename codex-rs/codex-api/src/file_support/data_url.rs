pub fn parse_data_url(url: &str) -> Option<(String, String)> {
    let url = url.strip_prefix("data:")?;
    let (header, data) = url.split_once(',')?;
    let mime = header.strip_suffix(";base64")?;
    Some((mime.to_string(), data.to_string()))
}

pub fn build_data_url(mime_type: &str, base64_data: &str) -> String {
    format!("data:{mime_type};base64,{base64_data}")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parse_roundtrip() {
        let url = build_data_url("application/pdf", "JVBERi0xLjQ=");
        let parsed = parse_data_url(&url);
        assert_eq!(
            parsed,
            Some(("application/pdf".to_string(), "JVBERi0xLjQ=".to_string()))
        );
    }

    #[test]
    fn parse_rejects_non_data_url() {
        assert_eq!(parse_data_url("https://example.com/file.pdf"), None);
    }
}
