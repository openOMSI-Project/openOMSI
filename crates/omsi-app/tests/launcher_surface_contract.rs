//! Guard against the launcher bypassing SurfaceState's sRGB presentation path.
//!
//! On ANGLE/D3D11 the renderer draws into an sRGB stand-in, while the actual
//! swapchain is UNORM. Resolving the launcher's 4x MSAA texture directly into
//! the swapchain causes a wgpu format validation error and a black window.

#[test]
fn launcher_uses_surface_state_for_frame_presentation() {
    // This is a source-level contract check, not a replacement for a Windows
    // ANGLE/WARP smoke test. The full launcher requires a real window.
    let source = include_str!("../src/launcher/mod.rs");
    let draw_frame = source
        .split_once("fn draw_frame(")
        .expect("launcher frame function")
        .1
        .split_once("fn check_exit(")
        .expect("end of launcher frame function")
        .0;
    let presentation = draw_frame
        .split_once("let frame = match surface.acquire()")
        .expect("frame acquired via SurfaceState")
        .1
        .split_once("self.shown =")
        .expect("end of presentation sequence")
        .0;

    assert!(
        presentation.contains("surface.view(&renderer.device, &frame)"),
        "launcher must draw through SurfaceState's sRGB-aware view"
    );
    assert!(
        presentation.contains("surface.present(&renderer.device, &renderer.queue, frame)"),
        "launcher must run SurfaceState's sRGB conversion before presenting"
    );
    assert!(!presentation.contains("frame.texture.create_view("));
    assert!(!presentation.contains("frame.present();"));
}
