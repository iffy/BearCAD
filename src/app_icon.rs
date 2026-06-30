//! Application window icon loaded from the bundled PNG asset.

use egui::IconData;
use std::sync::Arc;

const APP_ICON_PNG: &[u8] = include_bytes!("assets/appicon.png");

/// Window icons only need a modest raster; keep the bundled 1024px asset for packaging.
const WINDOW_ICON_SIZE: u32 = 128;

pub fn load() -> Arc<IconData> {
    match image::load_from_memory(APP_ICON_PNG) {
        Ok(image) => {
            let rgba = image::imageops::resize(
                &image.to_rgba8(),
                WINDOW_ICON_SIZE,
                WINDOW_ICON_SIZE,
                image::imageops::FilterType::Lanczos3,
            );
            let (width, height) = rgba.dimensions();
            Arc::new(IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            })
        }
        Err(_) => Arc::new(IconData::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_icon_png_decodes_to_square_image() {
        let image = image::load_from_memory(APP_ICON_PNG).expect("appicon.png should decode");
        assert!(image.width() >= 256);
        assert!(image.height() >= 256);
        assert_eq!(image.width(), image.height());
    }

    #[test]
    fn load_produces_window_sized_icon_data() {
        let icon = load();
        assert!(!icon.rgba.is_empty());
        assert_eq!(icon.width, WINDOW_ICON_SIZE);
        assert_eq!(icon.height, WINDOW_ICON_SIZE);
    }
}