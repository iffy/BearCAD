// BearCAD QuickLook Preview Extension (#1290, #1790).
//
// Space-bar QuickLook for `.bearcad` shows the colorful Home-orientation screenshot
// embedded in the SQLite `blobs` table (`kind = preview_png`) by the app on save.
// The retired interactive STL mesh preview is gone (#1790): the screenshot is the
// only preview.

import AppKit
import QuickLookUI
import SQLite3
import UniformTypeIdentifiers

final class PreviewViewController: NSViewController, QLPreviewingController {
    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
    }

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        // Heavy work off the main thread; UI setup back on main.
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Self.loadPreview(from: url)
            DispatchQueue.main.async {
                guard let self else {
                    handler(PreviewError.cancelled)
                    return
                }
                switch result {
                case .success(let image):
                    self.present(image)
                    handler(nil)
                case .failure(let err):
                    handler(err)
                }
            }
        }
    }

    // MARK: - Load

    private static func loadPreview(from url: URL) -> Result<NSImage, Error> {
        guard let png = readBlobBytes(path: url.path, kind: "preview_png"),
              let image = NSImage(data: png)
        else {
            return .failure(PreviewError.noPreview)
        }
        return .success(image)
    }

    /// Read a `blobs` row from a `.bearcad` SQLite file.
    private static func readBlobBytes(path: String, kind: String) -> Data? {
        var db: OpaquePointer?
        // Readonly — the previewed file may be on a network share / iCloud; don't write.
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let db else {
            return nil
        }
        defer { sqlite3_close(db) }

        let sql = "SELECT bytes FROM blobs WHERE kind = ?1 LIMIT 1"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
            return nil
        }
        defer { sqlite3_finalize(stmt) }

        let nsKind = kind as NSString
        sqlite3_bind_text(stmt, 1, nsKind.utf8String, -1, nil)
        guard sqlite3_step(stmt) == SQLITE_ROW else { return nil }
        guard let ptr = sqlite3_column_blob(stmt, 0) else { return nil }
        let len = Int(sqlite3_column_bytes(stmt, 0))
        return Data(bytes: ptr, count: len)
    }

    // MARK: - Present

    private func present(_ image: NSImage) {
        // Clear any prior content (reused controllers are rare but cheap to handle).
        view.subviews.forEach { $0.removeFromSuperview() }

        let imageView = NSImageView(frame: view.bounds)
        imageView.autoresizingMask = [.width, .height]
        imageView.imageScaling = .scaleProportionallyUpOrDown
        imageView.image = image
        view.addSubview(imageView)
    }
}

private enum PreviewError: LocalizedError {
    case noPreview
    case cancelled

    var errorDescription: String? {
        switch self {
        case .noPreview: return "No preview data in this BearCAD file (save it once in BearCAD)."
        case .cancelled: return "Preview cancelled."
        }
    }
}
