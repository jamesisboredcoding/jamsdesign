//! Roadmap Step 2's acceptance check: the Blobr main-window mockup builds from ops, with its
//! components, saves as a `.jdb` and renders.

#[allow(dead_code)]
#[path = "../examples/blobr_window.rs"]
mod example;

use blobr_mcp::{FileTarget, Target};
use jd_vector::NodeKind;

#[test]
fn blobr_window_builds_from_components_saves_and_renders() {
    let mut target = FileTarget::default();
    let win = example::build(&mut target).expect("ops apply");
    let doc = target.doc();
    let count = |f: &dyn Fn(&NodeKind) -> bool| doc.descendants(doc.pages()[0]).into_iter().filter(|&id| f(&doc.node(id).unwrap().kind)).count();
    assert!(count(&|k| matches!(k, NodeKind::Component { .. })) >= 20, "icons, tool buttons, tabs, rows, fields");
    assert_eq!(count(&|k| matches!(k, NodeKind::ComponentSet { .. })), 3);
    assert!(count(&|k| matches!(k, NodeKind::Instance { .. })) >= 40);

    let dir = std::env::temp_dir().join(format!("blobr-window-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("window.jdb");
    jd_vector::file::save(doc, &path).unwrap();
    let back = jd_vector::file::load(&path).unwrap();
    assert_eq!(back.descendants(back.pages()[0]).len(), doc.descendants(doc.pages()[0]).len());
    std::fs::remove_dir_all(&dir).ok();

    let png = jd_vector_render::render_png(doc, win, jd_vector_render::RenderOptions { scale: 0.25, ..Default::default() }).unwrap();
    assert!(png.len() > 1000);
}
