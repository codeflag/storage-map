//! Die Oberflaeche steckt komplett im Programm. Mit `--ui-dir` wird sie beim Entwickeln von der Platte gelesen.

macro_rules! asset {
    ($p:literal) => {
        ($p, include_bytes!(concat!("../ui/", $p)) as &[u8])
    };
}

const ASSETS: &[(&str, &[u8])] = &[
    asset!("index.html"),
    asset!("style.css"),
    asset!("js/app.js"),
    asset!("js/util.js"),
    asset!("js/charts.js"),
    asset!("js/treemap.js"),
    asset!("js/sunburst.js"),
    asset!("js/views.js"),
    asset!("js/views2.js"),
    asset!("js/views3.js"),
    asset!("favicon.svg"),
];

pub fn get(path: &str) -> Option<&'static [u8]> {
    ASSETS.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
