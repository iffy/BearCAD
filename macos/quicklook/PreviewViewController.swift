// BearCAD QuickLook Preview Extension (#1290).
//
// Space-bar QuickLook for `.bearcad` loads the binary STL snapshot from the SQLite
// `blobs` table (`kind = preview_stl`) and displays it in an SCNView with camera control
// so the user can rotate/pan/zoom like system STL previews.

import AppKit
import QuickLookUI
import SceneKit
import ModelIO
import SceneKit.ModelIO
import SQLite3
import UniformTypeIdentifiers

final class PreviewViewController: NSViewController, QLPreviewingController {
    private var scnView: SCNView?

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
                case .success(let payload):
                    self.present(payload)
                    handler(nil)
                case .failure(let err):
                    handler(err)
                }
            }
        }
    }

    // MARK: - Load

    private enum Payload {
        case mesh(SCNScene)
        case image(NSImage)
    }

    private static func loadPreview(from url: URL) -> Result<Payload, Error> {
        // Prefer the interactive mesh snapshot; fall back to the static PNG thumbnail.
        if let stl = readBlobBytes(path: url.path, kind: "preview_stl"), !stl.isEmpty {
            do {
                let scene = try sceneFromBinaryStl(stl)
                return .success(.mesh(scene))
            } catch {
                // Fall through to PNG.
            }
        }
        if let png = readBlobBytes(path: url.path, kind: "preview_png"),
           let image = NSImage(data: png)
        {
            return .success(.image(image))
        }
        return .failure(PreviewError.noPreview)
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

    /// Write the binary STL to a temp file and load via ModelIO → SceneKit.
    private static func sceneFromBinaryStl(_ stl: Data) throws -> SCNScene {
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("bearcad-ql-\(UUID().uuidString).stl")
        try stl.write(to: tmp)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let asset = MDLAsset(url: tmp)
        guard asset.count > 0 else {
            throw PreviewError.badMesh
        }
        let scene = SCNScene(mdlAsset: asset)
        // Soft fill so the model reads without a texture.
        scene.rootNode.enumerateChildNodes { node, _ in
            guard let geometry = node.geometry else { return }
            for material in geometry.materials {
                material.lightingModel = .blinn
                material.diffuse.contents = NSColor(calibratedRed: 0.55, green: 0.62, blue: 0.75, alpha: 1)
                material.metalness.contents = 0.05
                material.roughness.contents = 0.55
            }
        }
        return scene
    }

    // MARK: - Present

    private func present(_ payload: Payload) {
        // Clear any prior content (reused controllers are rare but cheap to handle).
        view.subviews.forEach { $0.removeFromSuperview() }
        scnView = nil

        switch payload {
        case .mesh(let scene):
            let scn = SCNView(frame: view.bounds)
            scn.autoresizingMask = [.width, .height]
            scn.scene = scene
            scn.allowsCameraControl = true // rotate / pan / zoom like system STL QuickLook
            scn.autoenablesDefaultLighting = true
            scn.backgroundColor = NSColor.windowBackgroundColor
            scn.antialiasingMode = .multisampling4X
            // Frame the mesh so it fills the view on first open.
            if let (center, radius) = boundingSphere(of: scene.rootNode) {
                let cameraNode = SCNNode()
                cameraNode.camera = SCNCamera()
                cameraNode.camera?.zNear = 0.01
                cameraNode.camera?.zFar = max(radius * 100, 1000)
                let distance = max(radius * 2.6, 1)
                cameraNode.position = SCNVector3(center.x + distance * 0.6,
                                                center.y + distance * 0.45,
                                                center.z + distance * 0.75)
                cameraNode.look(at: center)
                scene.rootNode.addChildNode(cameraNode)
                scn.pointOfView = cameraNode
            }
            view.addSubview(scn)
            scnView = scn

        case .image(let image):
            let imageView = NSImageView(frame: view.bounds)
            imageView.autoresizingMask = [.width, .height]
            imageView.imageScaling = .scaleProportionallyUpOrDown
            imageView.image = image
            view.addSubview(imageView)
        }
    }

    private func boundingSphere(of node: SCNNode) -> (SCNVector3, CGFloat)? {
        let (minV, maxV) = node.boundingBox
        let dx = maxV.x - minV.x
        let dy = maxV.y - minV.y
        let dz = maxV.z - minV.z
        if dx <= 0 && dy <= 0 && dz <= 0 { return nil }
        let center = SCNVector3(
            (minV.x + maxV.x) * 0.5,
            (minV.y + maxV.y) * 0.5,
            (minV.z + maxV.z) * 0.5
        )
        let radius = CGFloat(sqrt(dx * dx + dy * dy + dz * dz) * 0.5)
        return (center, max(radius, 0.001))
    }
}

private enum PreviewError: LocalizedError {
    case noPreview
    case badMesh
    case cancelled

    var errorDescription: String? {
        switch self {
        case .noPreview: return "No preview data in this BearCAD file (save it once in BearCAD)."
        case .badMesh: return "Could not load the embedded preview mesh."
        case .cancelled: return "Preview cancelled."
        }
    }
}
