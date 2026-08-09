mod decimal128 {
    #[path = "/Users/satishbabariya/Desktop/realm-rust/crates/realm-core-rs/src/decimal128/tables.rs"]
    pub mod tables;
    #[path = "/Users/satishbabariya/Desktop/realm-rust/crates/realm-core-rs/src/decimal128/conv.rs"]
    pub mod conv;
}
use std::io::{Read, Write};
extern "C" { #[link_name = "__bid_IDEC_glbround"] static mut GLBROUND: u32; }
fn main() {
    if let Ok(r) = std::env::var("ROUND") {
        unsafe { GLBROUND = r.parse().unwrap(); }
    }
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf).unwrap();
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for ch in buf.chunks_exact(8) {
        let bits = u64::from_le_bytes(ch.try_into().unwrap());
        let x = f64::from_bits(bits);
        let mut flags: u32 = 0;
        let r = decimal128::conv::binary64_to_bid128(x, &mut flags);
        writeln!(w, "{:016x} {:016x} {:016x} {}", bits, r[0], r[1], flags).unwrap();
    }
}
