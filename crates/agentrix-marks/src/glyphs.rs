//! Role glyphs: 24 px line icons from `concept/identity/marks.html`, plus the eight the sample
//! sites added for their desks (pin to leaf), drawn in the glyph colour.
//! Owner-drawn glyphs wait for the sealed-room cleaner (ADR-031, "Uploads").

/// Every role glyph: id, plain name, and the SVG inside a 24×24 viewBox.
pub const GLYPHS: &[(&str, &str, &str)] = &[
    (
        "cake",
        "Cake",
        r#"<path d="M4 20h16"/><path d="M5.5 20v-6.5h13V20"/><path d="M5.5 15.5c1.6 1.2 3.4 1.2 5 0 1.6 1.2 3.4 1.2 5 0 1 .8 2 .9 3 .5"/><path d="M12 13.5V10"/><path d="M12 7.6c-1-.9-1-2.1 0-3.4 1 1.3 1 2.5 0 3.4z"/>"#,
    ),
    (
        "loaf",
        "Loaf",
        r#"<path d="M6 12.2A3.6 3.6 0 0 1 6.8 5h10.4a3.6 3.6 0 0 1 .8 7.2V19a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1z"/><path d="M9.5 9.5l1.5 1.8"/><path d="M13 8.6l1.5 1.8"/>"#,
    ),
    (
        "bag",
        "Bag",
        r#"<path d="M5.5 8.5h13l-1.1 11.5H6.6z"/><path d="M9 8.5V7a3 3 0 0 1 6 0v1.5"/>"#,
    ),
    (
        "pen",
        "Pen",
        r#"<path d="M4.5 19.5l1-4L16 5l3 3L8.5 18.5z"/><path d="M14 7l3 3"/>"#,
    ),
    (
        "shield",
        "Shield",
        r#"<path d="M12 3.5l7 2.8v5.4c0 4.2-2.9 7.3-7 8.8-4.1-1.5-7-4.6-7-8.8V6.3z"/><path d="M9 12l2.1 2.1L15.2 10"/>"#,
    ),
    (
        "gift",
        "Gift",
        r#"<path d="M4 9.5h16v3.5H4z"/><path d="M5.5 13v7h13v-7"/><path d="M12 9.5V20"/><path d="M12 9.5C10.8 6.8 7.5 6 7.5 8c0 1.2 1.8 1.5 4.5 1.5z"/><path d="M12 9.5c1.2-2.7 4.5-3.5 4.5-1.5 0 1.2-1.8 1.5-4.5 1.5z"/>"#,
    ),
    (
        "perfume",
        "Perfume",
        r#"<path d="M9.5 3.5h5v3h-5z"/><path d="M10.5 6.5v2M13.5 6.5v2"/><path d="M6.5 11.5a3 3 0 0 1 3-3h5a3 3 0 0 1 3 3v7a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2z"/><path d="M9.5 14.5h5"/>"#,
    ),
    (
        "star",
        "Star",
        r#"<path d="M12 4l2.5 5.1 5.6.8-4 3.9.9 5.6L12 16.8l-5 2.6.9-5.6-4-3.9 5.6-.8z"/>"#,
    ),
    (
        "coin",
        "Coin",
        r#"<circle cx="12" cy="12" r="8"/><path d="M12 7.5v9"/><path d="M14.4 9.7c-.4-.8-1.3-1.3-2.4-1.3-1.3 0-2.4.7-2.4 1.8s1.1 1.4 2.4 1.7 2.4.7 2.4 1.8-1.1 1.8-2.4 1.8c-1.1 0-2-.5-2.4-1.3"/>"#,
    ),
    (
        "news",
        "Newspaper",
        r#"<path d="M4 5.5h12.5V19H6a2 2 0 0 1-2-2z"/><path d="M16.5 9H20v8a2 2 0 0 1-2 2h-1.5"/><path d="M7 9h6.5M7 12.5h6.5M7 16h4"/>"#,
    ),
    (
        "calendar",
        "Calendar",
        r#"<rect x="4" y="5.5" width="16" height="14.5" rx="2"/><path d="M4 10h16M8.5 3.5v4M15.5 3.5v4"/><path d="M8 14h3"/>"#,
    ),
    (
        "key",
        "Key",
        r#"<circle cx="8.5" cy="15.5" r="4"/><path d="M11.3 12.7L19.5 4.5"/><path d="M16.5 7.5l2.2 2.2M14.5 9.5l1.8 1.8"/>"#,
    ),
    (
        "boxes",
        "Boxes",
        r#"<path d="M3.5 7.5L12 3.5l8.5 4L12 11.5z"/><path d="M3.5 7.5v9l8.5 4 8.5-4v-9"/><path d="M12 11.5v9"/>"#,
    ),
    (
        "chat",
        "Speech bubble",
        r#"<path d="M5 5h14a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 16h-6.5L8 19.5V16H5a1.5 1.5 0 0 1-1.5-1.5v-8A1.5 1.5 0 0 1 5 5z"/>"#,
    ),
    (
        "pin",
        "Pin",
        r#"<path d="M12 21s-6-5.5-6-11a6 6 0 0 1 12 0c0 5.5-6 11-6 11z"/><circle cx="12" cy="10" r="2"/>"#,
    ),
    (
        "search",
        "Search",
        r#"<circle cx="11" cy="11" r="6"/><path d="M20 20l-4-4"/>"#,
    ),
    (
        "bottle",
        "Bottle",
        r#"<path d="M10 3h4v3l2 2v11a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2V8l2-2z"/><path d="M8 12h8"/>"#,
    ),
    (
        "book",
        "Book",
        r#"<path d="M5 4h9a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z"/><path d="M5 17a3 3 0 0 1 3-3h9"/>"#,
    ),
    (
        "archive",
        "Archive box",
        r#"<path d="M4 5h16v4H4z"/><path d="M5 9v10h14V9"/><path d="M10 13h4"/>"#,
    ),
    (
        "ticket",
        "Ticket",
        r#"<path d="M4 7h16v3a2 2 0 0 0 0 4v3H4v-3a2 2 0 0 0 0-4z"/><path d="M14 7v10"/>"#,
    ),
    (
        "train",
        "Train",
        r#"<rect x="6" y="3" width="12" height="13" rx="3"/><path d="M6 10h12"/><path d="M9 20l-2 1M15 20l2 1"/><circle cx="9" cy="13" r=".8"/><circle cx="15" cy="13" r=".8"/>"#,
    ),
    (
        "leaf",
        "Leaf",
        r#"<path d="M5 19c0-8 5-13 14-14-1 9-6 14-14 14z"/><path d="M5 19l7-7"/>"#,
    ),
];

/// The SVG for a role glyph, if the id is known.
pub fn glyph_svg(id: &str) -> Option<&'static str> {
    GLYPHS
        .iter()
        .find(|(g, _, _)| *g == id)
        .map(|(_, _, svg)| *svg)
}

/// The plain name of a role glyph, for labels in the editor.
pub fn glyph_name(id: &str) -> Option<&'static str> {
    GLYPHS.iter().find(|(g, _, _)| *g == id).map(|(_, n, _)| *n)
}
