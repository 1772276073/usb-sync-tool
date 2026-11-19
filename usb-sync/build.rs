//! 构建脚本：把 assets/app.ico 嵌入 Windows exe（GNU 工具链走 windres，需在 PATH）
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.compile().expect("嵌入图标失败（需要 windres 在 PATH 中，见 build.cmd）");
    }
    println!("cargo:rerun-if-changed=assets/app.ico");
}
