fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon_path = std::path::Path::new("target/generated/appicon.ico");
    if let Some(parent) = icon_path.parent() {
        std::fs::create_dir_all(parent).expect("create generated icon directory");
    }
    png_to_ico("src/assets/appicon.png", icon_path);

    let icon_path = icon_path
        .to_str()
        .expect("generated icon path should be valid UTF-8");
    let mut res = winres::WindowsResource::new();
    res.set_icon(icon_path);
    res.compile().expect("compile Windows icon resources");
}

fn png_to_ico(png_path: &str, out_path: &std::path::Path) {
    use ico::{IconDir, IconImage};
    use image::imageops::FilterType;
    use std::fs::File;
    use std::io::BufWriter;

    let image = image::ImageReader::open(png_path)
        .expect("open app icon png")
        .decode()
        .expect("decode app icon png")
        .into_rgba8();

    let mut icon_dir = IconDir::new(ico::ResourceType::Icon);
    for size in [256u32, 48, 32, 16] {
        let resized = image::imageops::resize(&image, size, size, FilterType::Lanczos3);
        let (width, height) = resized.dimensions();
        let icon = IconImage::from_rgba_data(width, height, resized.into_raw());
        let entry = ico::IconDirEntry::encode(&icon).expect("encode icon size");
        icon_dir.add_entry(entry);
    }

    let file = File::create(out_path).expect("create ico file");
    icon_dir
        .write(BufWriter::new(file))
        .expect("write ico file");
}