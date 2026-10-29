fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = match args.next().as_deref() {
        None => None,
        Some("--input") => Some(std::path::PathBuf::from(
            args.next().ok_or("Provide a file after --input.")?,
        )),
        _ => return Err("Usage: pnpm catalog:update [--input <upstream-json>]".into()),
    };
    if args.next().is_some() {
        return Err("Unexpected catalogue update argument.".into());
    }
    tauri::async_runtime::block_on(wackcode_lib::model_catalog::update_bundle(input.as_deref()))
}
