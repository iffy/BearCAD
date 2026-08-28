//! Headless mode: the full app — egui pass loop, 3D viewport, script runner — driven
//! by hand against an **offscreen** wgpu target, with no winit window and no OS event
//! loop. `--headless` on the CLI; the default for `--script`.
//!
//! eframe can't do this itself (its wgpu backend is welded to a window surface), so
//! this module re-implements the thin backend layer eframe normally provides:
//!
//! 1. build a surfaceless [`egui_wgpu::RenderState`] (adapter + device + egui renderer,
//!    `compatible_surface: None` — egui falls back to `Rgba8Unorm`, a gamma-space
//!    format, which is what egui prefers);
//! 2. each frame, assemble [`egui::RawInput`] (fixed screen rect, no OS events), run
//!    the app through [`egui::Context::run_ui`], tessellate, and render into an
//!    offscreen texture — paint callbacks (the 3D viewport) execute inside that pass
//!    exactly as on screen;
//! 3. service the two [`egui::ViewportCommand`]s that only a backend can fulfil:
//!    [`egui::ViewportCommand::Close`] ends the run, and
//!    [`egui::ViewportCommand::Screenshot`] captures the frame and delivers it back as
//!    an [`egui::Event::Screenshot`] on the next pass — the same contract the windowed
//!    backend honors, which is how `bearcad._screenshot` keeps working unchanged.

use crate::native_menu::NativeMenu;
use crate::script::{ScriptOptions, ScriptRunner};
use crate::App;
use eframe::egui;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// The headless "window" size in points. `BEARCAD_WINDOW` overrides it, so the
/// interaction-test harness keeps pinning layout the same way it does for windows.
const DEFAULT_SIZE: [f32; 2] = [1280.0, 800.0];

/// Run the app headlessly. `Err` when GPU setup fails or the script fails — the CLI
/// turns that into a non-zero exit, matching the windowed `--exit` contract (#125).
pub fn run(script_opts: ScriptOptions) -> Result<(), String> {
    // Watchdog for unattended runs (#61): same contract as the windowed path.
    if let Some(secs) = script_opts.timeout_secs {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            eprintln!("error: bearcad did not exit within {secs}s, forcing exit");
            std::process::exit(1);
        });
    }

    let script = if script_opts.repl {
        if script_opts.script_path.is_some() {
            return Err("--repl and --script are mutually exclusive".to_string());
        }
        Some(ScriptRunner::repl()).transpose()
    } else {
        script_opts
            .script_path
            .as_ref()
            .map(|p| ScriptRunner::from_file(Path::new(p)))
            .transpose()
    }
    .map_err(|e| e.to_string())?;

    // Headless always terminates the process, so it adopts the `--exit` contract:
    // script completion (or an uncaught error) closes the run and fails it (#125).
    // With no script there is nothing to keep a headless process alive for — open,
    // run startup actions, quit.
    let exit_on_complete = true;

    let size = crate::window_size_override().unwrap_or(DEFAULT_SIZE);

    // Surfaceless GPU setup: the same construction eframe runs for a window, minus the
    // surface. Without one, egui falls back to `Rgba8Unorm` — a gamma-space format,
    // which is the format egui prefers, so UI and viewport colors match the window.
    let config = egui_wgpu::WgpuConfiguration::default();
    // The default setup's descriptor is the env-driven, display-less one; build the
    // instance from it directly (InstanceDescriptor isn't Clone).
    let instance = match &config.wgpu_setup {
        egui_wgpu::WgpuSetup::CreateNew(_) => {
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
        }
        egui_wgpu::WgpuSetup::Existing(existing) => existing.instance.clone(),
    };
    let render_state = pollster::block_on(egui_wgpu::RenderState::create(
        &config,
        &instance,
        None,
        egui_wgpu::RendererOptions::default(),
    ))
    .map_err(|e| format!("headless GPU initialization failed: {e}"))?;
    {
        let info = render_state.adapter.get_info();
        crate::diag::info(format!(
            "headless: {:?} on {} ({:?})",
            info.backend, info.name, info.device_type
        ));
    }

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);

    // The look-alikes eframe would hand the app. The `_new_kittest` constructors are
    // egui's own sanctioned way to run an `App` without a window; only the render
    // state needs filling in.
    let mut cc = eframe::CreationContext::_new_kittest(ctx.clone());
    cc.wgpu_render_state = Some(render_state.clone());
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(render_state.clone());

    // Build the menu bar unattached so `bearcad.ui.menu_structure()` describes the same
    // bar headless as windowed (#1622). muda's macOS menu needs the main thread (the
    // real headless run is on it; unit tests are not), and platforms whose menu toolkit
    // can't start without a display degrade gracefully — scripts don't drive the OS
    // menu by pointer anyway.
    #[cfg(target_os = "macos")]
    let can_build_menu = objc2::MainThreadMarker::new().is_some();
    #[cfg(not(target_os = "macos"))]
    let can_build_menu = true;
    let native_menu = if can_build_menu {
        match NativeMenu::install_unattached(&cc) {
            Ok(menu) => Some(menu),
            Err(err) => {
                crate::diag::warn(format!("headless: native menu unavailable ({err})"));
                None
            }
        }
    } else {
        None
    };
    let script_failed = Arc::new(AtomicBool::new(false));
    let script_failed_for_app = script_failed.clone();
    let mut app = App::new(
        &cc,
        script,
        script_opts.document_path,
        exit_on_complete,
        script_opts.show_commands,
        native_menu,
        script_failed_for_app,
    );
    app.set_headless(true);
    if script_opts.rebuild {
        app.state.apply(crate::actions::Action::ForceRebuildGeometry);
    }
    if let Some(index) = script_opts
        .tutorial
        .as_deref()
        .and_then(crate::tutorial::tutorial_index)
    {
        app.state.apply(crate::actions::Action::StartTutorial { index });
    }

    // egui's reactive frame pacing, reimplemented: `request_repaint` (app code, REPL
    // stdin reader, timers) flips a flag and wakes the loop; otherwise the loop parks
    // on a short poll so time-based things still tick.
    let repaint = Arc::new((Mutex::new(false), Condvar::new()));
    {
        let repaint = repaint.clone();
        ctx.set_request_repaint_callback(move |_info| {
            let (flag, cvar) = &*repaint;
            let mut pending = flag.lock().unwrap();
            *pending = true;
            cvar.notify_all();
        });
    }

    let start = crate::time::Instant::now();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(size));
    let mut offscreen = Offscreen::new(&render_state, size);
    let mut pending_screenshots: Vec<egui::Event> = Vec::new();
    let mut finishing = false;

    loop {
        if !*repaint.0.lock().unwrap() {
            // Nothing pending: park briefly. Timers and the REPL reader wake us early.
            let (flag, cvar) = &*repaint;
            let guard = flag.lock().unwrap();
            let _ = cvar
                .wait_timeout_while(guard, std::time::Duration::from_millis(100), |p| !*p)
                .unwrap();
        }
        *repaint.0.lock().unwrap() = false;

        let mut raw = egui::RawInput {
            time: Some(start.elapsed().as_secs_f64()),
            screen_rect: Some(screen),
            ..Default::default()
        };
        raw.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                inner_rect: Some(screen),
                outer_rect: Some(screen),
                focused: Some(true),
                ..Default::default()
            },
        );
        // Screenshot replies from the previous frame, then scripted input — the same
        // ordering egui_winit uses when it fills a frame's input.
        raw.events.append(&mut pending_screenshots);
        eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);

        let full = ctx.run_ui(raw, |ui| eframe::App::ui(&mut app, ui, &mut frame));

        // The two commands only a backend can service. Everything else (title,
        // maximize, focus) is meaningless without a window and ignored.
        let mut close = false;
        let mut screenshot_requests = Vec::new();
        for output in full.viewport_output.values() {
            for command in &output.commands {
                match command {
                    egui::ViewportCommand::Close => close = true,
                    egui::ViewportCommand::Screenshot(user_data) => {
                        screenshot_requests.push(user_data.clone());
                    }
                    _ => {}
                }
            }
        }

        let captured = offscreen.paint(&ctx, full);
        for user_data in screenshot_requests {
            if let Some(image) = &captured {
                pending_screenshots.push(egui::Event::Screenshot {
                    viewport_id: egui::ViewportId::ROOT,
                    user_data,
                    image: Arc::new(image.clone()),
                });
            }
        }

        // Close comes from `bearcad.quit()`, `--exit`, or exit-after-startup. A finished
        // script with no `--exit` still ends the run: there is no window to stay open for.
        // The break is deferred one frame — the frame where the script finishes is also
        // the one where `tick_script` stores the failure flag and sends the close, so
        // breaking immediately would skip that bookkeeping. The flag is sticky because
        // the close command itself is gone by the next frame.
        if finishing {
            break;
        }
        if close || app.script_finished() {
            finishing = true;
        }
    }

    if script_failed.load(Ordering::SeqCst) {
        return Err(app.state.status.clone());
    }
    Ok(())
}

/// The offscreen render target and the painting half of a minimal egui-wgpu backend.
struct Offscreen {
    render_state: egui_wgpu::RenderState,
    texture: wgpu::Texture,
    /// Physical pixel size of the target.
    size_px: [u32; 2],
}

impl Offscreen {
    fn new(render_state: &egui_wgpu::RenderState, size_points: [f32; 2]) -> Self {
        // 1 point = 1 pixel: screenshots come out at CSS scale, like CI's Xvfb runs.
        let size_px = [size_points[0] as u32, size_points[1] as u32];
        let texture = Self::create_texture(&render_state.device, render_state.target_format, size_px);
        Self {
            render_state: render_state.clone(),
            texture,
            size_px,
        }
    }

    fn create_texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size_px: [u32; 2],
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bearcad_headless_target"),
            size: wgpu::Extent3d {
                width: size_px[0],
                height: size_px[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Tessellate and render one frame into the offscreen target. Returns the captured
    /// image (only read back when a screenshot was requested — the copy + map is not free).
    fn paint(&mut self, ctx: &egui::Context, full: egui::FullOutput) -> Option<egui::ColorImage> {
        let wants_capture = full
            .viewport_output
            .values()
            .any(|out| out.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Screenshot(_))));
        let clipped = ctx.tessellate(full.shapes, full.pixels_per_point);
        let device = &self.render_state.device;
        let queue = &self.render_state.queue;

        {
            let mut renderer = self.render_state.renderer.write();
            for (id, delta) in &full.textures_delta.set {
                renderer.update_texture(device, queue, *id, delta);
            }
        }

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size_px,
            pixels_per_point: full.pixels_per_point,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bearcad_headless_encoder"),
        });
        let user_cmd_bufs = {
            let mut renderer = self.render_state.renderer.write();
            renderer.update_buffers(device, queue, &mut encoder, &clipped, &screen)
        };

        let view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bearcad_headless_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The 3D viewport's paint callbacks execute here, mid-pass, exactly as they
            // do on screen.
            self.render_state
                .renderer
                .read()
                .render(&mut render_pass.forget_lifetime(), &clipped, &screen);
        }

        let encoded = encoder.finish();
        queue.submit(std::iter::chain(user_cmd_bufs, [encoded]));

        // Free after submit, like the windowed backend — the frame may still reference them.
        {
            let mut renderer = self.render_state.renderer.write();
            for id in &full.textures_delta.free {
                renderer.free_texture(id);
            }
        }

        if wants_capture {
            Some(self.read_back())
        } else {
            None
        }
    }

    /// Copy the target texture to a buffer and read it back synchronously — headless
    /// has no frames to hide an async read behind, and correctness beats latency here.
    fn read_back(&self) -> egui::ColorImage {
        let device = &self.render_state.device;
        let queue = &self.render_state.queue;
        let [w, h] = self.size_px;
        let bytes_per_row = ((w * 4 + 255) / 256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_headless_readback"),
            size: (bytes_per_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bearcad_headless_copy"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);

        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("headless readback poll failed");
        rx.recv()
            .expect("readback callback ran")
            .expect("headless screenshot readback failed");

        let data = buffer.slice(..).get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for row in 0..h {
            let start = (row * bytes_per_row) as usize;
            let row_bytes = &data[start..start + (w * 4) as usize];
            for rgba in row_bytes.chunks_exact(4) {
                pixels.push(egui::Color32::from_rgba_unmultiplied(
                    rgba[0], rgba[1], rgba[2], rgba[3],
                ));
            }
        }
        drop(data);
        buffer.unmap();
        egui::ColorImage::new([w as usize, h as usize], pixels)
    }
}
