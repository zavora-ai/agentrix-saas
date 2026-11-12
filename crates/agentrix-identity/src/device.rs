//! A plain device name from the `User-Agent` header, for the sessions list and the new-sign-in
//! alert: "Chrome on a Mac", "Safari on an iPhone". Nothing else is derived from it.

/// The device a browser describes, in words a customer reads.
pub fn describe(user_agent: Option<&str>) -> String {
    let Some(ua) = user_agent.filter(|u| !u.trim().is_empty()) else {
        return "A device we couldn't name".into();
    };
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("OPR/") || ua.contains("Opera") {
        "Opera"
    } else if ua.contains("SamsungBrowser") {
        "Samsung Internet"
    } else if ua.contains("Firefox/") || ua.contains("FxiOS") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("CriOS") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "A browser"
    };
    let system = if ua.contains("iPhone") {
        Some("an iPhone")
    } else if ua.contains("iPad") {
        Some("an iPad")
    } else if ua.contains("Android") {
        Some("an Android phone")
    } else if ua.contains("Windows") {
        Some("a Windows PC")
    } else if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        Some("a Mac")
    } else if ua.contains("CrOS") {
        Some("a Chromebook")
    } else if ua.contains("Linux") {
        Some("a Linux computer")
    } else {
        None
    };
    match system {
        Some(system) => format!("{browser} on {system}"),
        None => browser.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn names_common_devices() {
        let mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
        assert_eq!(describe(Some(mac)), "Chrome on a Mac");
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
        assert_eq!(describe(Some(iphone)), "Safari on an iPhone");
        let win = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0";
        assert_eq!(describe(Some(win)), "Edge on a Windows PC");
        assert_eq!(describe(None), "A device we couldn't name");
    }
}
