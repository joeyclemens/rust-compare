use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Get the output directory
    let out_dir = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir).parent().unwrap().parent().unwrap().parent().unwrap();

    // Create static directory in the output directory
    let static_dir = out_dir.join("static");
    fs::create_dir_all(&static_dir).unwrap();

    // Copy styles.css to the static directory
    let source_css = Path::new("static/styles.css");
    let target_css = static_dir.join("styles.css");
    if source_css.exists() {
        fs::copy(source_css, target_css).unwrap();
    }
} 