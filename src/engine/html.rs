//! ---- ws:html-layer ----
//! HTML + CSS layers and scenes: the template format (`library/README.md`), and the renderer that
//! drives a headless Microsoft Edge over its DevTools connection (method and measurements: the
//! "creator-kit spike" entry in notes.md).

/// An icon from the library (`library/icons/*.svg`, drawn with `currentColor`) as an HTML layer page
/// with a colour field, so an icon is placed, coloured and rendered like any other HTML layer.
pub fn icon_page(name: &str, svg: &str) -> String {
    let name = name.replace(['<', '>', '&', '"'], " ");
    let svg = svg.trim_start_matches('\u{feff}');
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{name}</title>\n\
         <meta name=\"se:kind\" content=\"layer\">\n<meta name=\"se:desc\" content=\"Icon\">\n\
         <meta name=\"se:tags\" content=\"icon\">\n<meta name=\"se:size\" content=\"512x512\">\n\
         <meta name=\"se:duration\" content=\"4s\">\n<meta name=\"se:field\" content=\"color | color | #ffffff\">\n\
         <style>\nhtml, body {{ margin: 0; background: transparent; overflow: hidden; }}\n\
         body {{ color: {{{{color}}}}; }}\n\
         svg {{ display: block; width: 448px; height: 448px; margin: 32px; \
         filter: drop-shadow(0 6px 14px rgba(0, 0, 0, .45)); }}\n</style>\n</head>\n<body>{svg}</body>\n</html>\n"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_icon_becomes_a_page_with_a_colour_field() {
        let page = super::icon_page("Like <3", "<svg viewBox=\"0 0 24 24\"><path d=\"M0 0\"/></svg>");
        assert!(page.contains("color: {{color}};") && page.contains("se:field\" content=\"color | color | #ffffff\""));
        assert!(page.contains("<title>Like  3</title>") && page.contains("<path d=\"M0 0\"/>"));
        assert!(!page.to_ascii_lowercase().contains("<script"));
    }
}
