//! Run the dark-logo pipeline on a host machine, where there is no image codec.
//!
//! The pipeline is pure maths over a `Raster`; only decoding and encoding need
//! Android. So this takes pixels that something else decoded and hands back the
//! pixels of every treatment, in the one framing the JNI codec already uses
//! (`CarnyxNet.HEADER`): a big-endian `u32` width, a big-endian `u32` height,
//! then straight RGBA.
//!
//! ```sh
//! cargo run --release --example darkprobe -- in.rgba out-prefix [RRGGBB]
//! ```
//!
//! Writes `out-prefix.<TREATMENT>.rgba` for every candidate and prints one line
//! per fact on stdout: the pick, the coverage, the key's note, each gate's note.
//! The background defaults to the dark card the face composites onto.
//! `tools/dark-sheet.py` drives this over a folder of logos and lays the results
//! out on that card, which is how #143 was looked at rather than reasoned about.

use carnyx::logos::dark::pipeline::adapt_logo_for_dark;
use carnyx::logos::dark::LOGO_DARK_BG;
use carnyx::logos::Raster;

fn read(path: &str) -> Raster {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert!(b.len() >= 8, "{path}: no header");
    let w = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let h = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    assert_eq!(b.len(), 8 + (w as usize) * (h as usize) * 4, "{path}: size does not match its header");
    Raster { w, h, rgba: b[8..].to_vec() }
}

fn write(path: &str, r: &Raster) {
    let mut out = Vec::with_capacity(8 + r.rgba.len());
    out.extend_from_slice(&r.w.to_be_bytes());
    out.extend_from_slice(&r.h.to_be_bytes());
    out.extend_from_slice(&r.rgba);
    std::fs::write(path, out).unwrap_or_else(|e| panic!("{path}: {e}"));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: darkprobe in.rgba out-prefix [RRGGBB]");
        std::process::exit(2);
    }
    let bg = match args.get(2) {
        Some(hex) => {
            let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).expect("RRGGBB");
            [
                ((n >> 16) & 255) as f64 / 255.0,
                ((n >> 8) & 255) as f64 / 255.0,
                (n & 255) as f64 / 255.0,
            ]
        }
        None => LOGO_DARK_BG,
    };
    let src = read(&args[0]);
    let started = std::time::Instant::now();
    let res = adapt_logo_for_dark(&src, bg);
    let ms = started.elapsed().as_millis();
    println!("pick {}", res.pick.as_enum());
    println!("coverage {:.3}", res.coverage);
    println!("key {}", res.key.note);
    println!("ms {ms}");
    for (t, note) in &res.gates {
        println!("gate {} {note}", t.as_enum());
    }
    for c in &res.candidates {
        write(&format!("{}.{}.rgba", args[1], c.treatment.as_enum()), &c.raster);
    }
}
