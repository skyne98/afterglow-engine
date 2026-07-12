use std::fs;
fn main() {
    let data = fs::read("crates/afterglow-web/www/checker.basis").unwrap();
    println!("Basis file: {} bytes", data.len());

    match afterglow_texture::transcode(&data, afterglow_texture::FORMAT_RGBA) {
        Ok(out) => {
            let count = u32::from_le_bytes(out[0..4].try_into().unwrap());
            println!("RGBA: {} bytes, {count} mips", out.len());
            let mut off: usize = 4;
            for i in 0..count {
                let w = u32::from_le_bytes(out[off..off+4].try_into().unwrap()); off += 4;
                let h = u32::from_le_bytes(out[off..off+4].try_into().unwrap()); off += 4;
                let len = u32::from_le_bytes(out[off..off+4].try_into().unwrap()); off += 4;
                let first8 = &out[off..(off + 8).min(off + len as usize)];
                println!("  mip {i}: {w}x{h}, {len} bytes, first bytes: {:02x?}", first8);
                off += len as usize;
            }
        }
        Err(e) => println!("RGBA error: {e}"),
    }
}
