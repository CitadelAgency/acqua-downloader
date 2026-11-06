pub fn file_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut name: String = name.chars().filter(|c| !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}')).map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) || c == '\u{fffd}' { '_' } else { c }).collect::<String>().trim_start_matches(' ').trim_end_matches(['.', ' ']).into();
    if name.is_empty() {
        name = "download".into();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|s| s.chars().count() == 1 && "0123456789¹²³".contains(s))
    });
    if reserved {
        name.insert(0, '_');
    }
    if name.encode_utf16().count() > 240 {
        let ext = std::path::Path::new(&name)
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| s.encode_utf16().count() < 32)
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        let budget = 240 - ext.encode_utf16().count();
        let mut count = 0;
        name = name
            .chars()
            .take_while(|c| {
                count += c.len_utf16();
                count <= budget
            })
            .collect::<String>()
            .trim_end_matches(['.', ' '])
            .to_string()
            + &ext;
    }
    name
}
pub fn response_name(disposition: &str, url: &url::Url) -> String {
    let mut basic = None;
    for field in disposition.split(';').skip(1) {
        if let Some((key, value)) = field.trim().split_once('=') {
            let value = value.trim().trim_matches('"');
            if key.eq_ignore_ascii_case("filename*") {
                if let Some((charset, rest)) = value.split_once('\'') {
                    if charset.eq_ignore_ascii_case("utf-8") {
                        if let Some((_, raw)) = rest.split_once('\'') {
                            return file_name(
                                &percent_encoding::percent_decode_str(raw).decode_utf8_lossy(),
                            );
                        }
                    }
                }
            }
            if key.eq_ignore_ascii_case("filename") {
                basic = Some(value);
            }
        }
    }
    if let Some(s) = basic {
        return file_name(s);
    }
    let segment = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| url.host_str().unwrap_or("download"));
    file_name(&percent_encoding::percent_decode_str(segment).decode_utf8_lossy())
}
