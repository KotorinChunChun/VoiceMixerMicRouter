fn main() {
    println!("cargo:rerun-if-changed=icons/mic.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        winresource::WindowsResource::new()
            .set("ProductName", "VoiceMixerMicRouter")
            .set("FileDescription", "VoiceMixerMicRouter")
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .set("OriginalFilename", "VoiceMixerMicRouter.exe")
            .set_icon("icons/mic.ico")
            .compile()
            .expect("Windows製品資源の生成に失敗しました");
    }
}
