//! Fonts compiled into the binary.  The PDF pipeline and the UI both use
//! them, so a read-only install still renders CJK correctly.

pub fn embedded() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "NotoSansSC-Regular.otf",
            include_bytes!("../assets/fonts/NotoSansSC-Regular.otf").to_vec(),
        ),
        (
            "NotoSansSC-Bold.otf",
            include_bytes!("../assets/fonts/NotoSansSC-Bold.otf").to_vec(),
        ),
        (
            "NotoSerifSC-Regular.otf",
            include_bytes!("../assets/fonts/NotoSerifSC-Regular.otf").to_vec(),
        ),
        (
            "NotoSerifSC-Bold.otf",
            include_bytes!("../assets/fonts/NotoSerifSC-Bold.otf").to_vec(),
        ),
    ]
}
