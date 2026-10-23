//! Leitor minimo de .cur (entradas BMP) para gerar previas.
//! Implementacao original. PNG comprimido e .ani retornam None.

use std::path::Path;

pub struct CurImage {
    pub width: u32,
    pub height: u32,
    /// RGBA, de cima para baixo.
    pub rgba: Vec<u8>,
}

fn u16le(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32le(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn i32le(d: &[u8], o: usize) -> Option<i32> {
    d.get(o..o + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub fn parse_cur_file(path: &Path) -> Option<CurImage> {
    std::fs::read(path).ok().and_then(|d| parse_cur_bytes(&d))
}

pub fn parse_cur_bytes(d: &[u8]) -> Option<CurImage> {
    // Tipo 2 = cursor, tipo 1 = icone (o Windows aceita ambos como cursor).
    let kind = u16le(d, 2)?;
    if d.len() < 6 || (kind != 1 && kind != 2) {
        return None;
    }
    let count = u16le(d, 4)? as usize;
    if count == 0 || d.len() < 6 + count * 16 {
        return None;
    }
    // Maior area vence.
    let mut best: Option<(u32, u32, usize)> = None; // (area, bytes, offset)
    for i in 0..count {
        let o = 6 + i * 16;
        let w = d[o] as u32;
        let w = if w == 0 { 256 } else { w };
        let bytes = u32le(d, o + 8)?;
        let off = u32le(d, o + 12)? as usize;
        let area = w * w;
        if best.map(|(a, _, _)| area > a).unwrap_or(true) {
            best = Some((area, bytes, off));
        }
    }
    let (_, bytes, off) = best?;
    let img = d.get(off..off + bytes as usize)?;
    decode_image(img)
}

/// Decodifica uma imagem de cursor/icone em formato BMP (BITMAPINFOHEADER +
/// paleta + XOR + AND). Usado por .cur e por frames .ani.
pub fn decode_image(img: &[u8]) -> Option<CurImage> {
    if img.len() >= 8 && img[0..8] == *b"\x89PNG\r\n\x1a\n" {
        let dyn_img = image::load_from_memory(img).ok()?;
        let rgba = dyn_img.to_rgba8();
        return Some(CurImage {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        });
    }
    if img.len() < 40 || u32le(img, 0)? != 40 {
        return None;
    }
    let w = i32le(img, 4)?;
    let h2 = i32le(img, 8)?;
    let bpp = u16le(img, 14)? as u32;
    if w <= 0 || h2 <= 0 || u32le(img, 16)? != 0 {
        return None; // So BI_RGB.
    }
    if !matches!(bpp, 1 | 4 | 8 | 24 | 32) {
        return None;
    }
    let w = w as u32;
    let h = (h2 as u32) / 2;
    if h == 0 || w > 256 || h > 256 {
        return None;
    }
    let pal_n = if bpp <= 8 { 1u32 << bpp } else { 0 };
    let xor_off = 40 + (pal_n * 4) as usize;
    let row_bytes = (((bpp * w + 31) / 32) * 4) as usize;
    let xor_len = row_bytes * h as usize;
    let xor = img.get(xor_off..xor_off + xor_len)?;
    let and_row = (((w + 31) / 32) * 4) as usize;
    let and = img.get(xor_off + xor_len..xor_off + xor_len + and_row * h as usize)?;

    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b, mut a) = match bpp {
                32 => {
                    let o = y as usize * row_bytes + x as usize * 4;
                    (xor[o + 2], xor[o + 1], xor[o], xor[o + 3])
                }
                24 => {
                    let o = y as usize * row_bytes + x as usize * 3;
                    (xor[o + 2], xor[o + 1], xor[o], 255)
                }
                _ => {
                    let bits = match bpp {
                        8 => {
                            let o = y as usize * row_bytes + x as usize;
                            xor[o] as u32
                        }
                        4 => {
                            let o = y as usize * row_bytes + x as usize / 2;
                            let v = xor[o];
                            if x % 2 == 0 { (v >> 4) as u32 } else { (v & 0x0f) as u32 }
                        }
                        _ => {
                            let o = y as usize * row_bytes + x as usize / 8;
                            let bit = 7 - (x % 8);
                            ((xor[o] >> bit) & 1) as u32
                        }
                    };
                    let p = (bits * 4) as usize;
                    let q = img.get(40 + p..40 + p + 4)?;
                    (q[2], q[1], q[0], 255)
                }
            };
            let ao = y as usize * and_row + x as usize / 8;
            if (and[ao] >> (7 - (x % 8))) & 1 == 1 {
                a = 0;
            }
            // Linhas BMP sao de baixo para cima.
            let dy = h - 1 - y;
            let o = ((dy * w + x) * 4) as usize;
            rgba[o..o + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    Some(CurImage { width: w, height: h, rgba })
}

/// Primeiro frame de um .ani (RIFF ACON): procura o primeiro chunk 'icon'.
/// Implementacao original, sem dependencias.
pub fn parse_ani_first_frame(d: &[u8]) -> Option<CurImage> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"ACON" {
        return None;
    }
    find_icon_chunk(&d[12..]).and_then(decode_ani_icon)
}

/// Chunk 'icon' do ANI: um diretorio de icone (ICONDIR + entradas) + imagem.
/// Os offsets das entradas sao relativos ao inicio do chunk.
fn decode_ani_icon(chunk: &[u8]) -> Option<CurImage> {
    if chunk.len() < 22 {
        return None;
    }
    let kind = u16le(chunk, 2)?;
    if kind != 1 && kind != 2 {
        return None;
    }
    let count = u16le(chunk, 4)? as usize;
    if count == 0 || chunk.len() < 6 + count * 16 {
        return None;
    }
    let mut best: Option<(u32, u32, usize)> = None;
    for i in 0..count {
        let o = 6 + i * 16;
        let w = chunk[o] as u32;
        let w = if w == 0 { 256 } else { w };
        let bytes = u32le(chunk, o + 8)? as usize;
        let off = u32le(chunk, o + 12)? as usize;
        let area = w * w;
        if best.map(|(a, _, _)| area > a).unwrap_or(true) {
            best = Some((area, bytes as u32, off));
        }
    }
    let (_, bytes, off) = best?;
    let img = chunk.get(off..off + bytes as usize)?;
    decode_image(img)
}

fn find_icon_chunk(d: &[u8]) -> Option<&[u8]> {
    let mut pos = 0;
    while pos + 8 <= d.len() {
        let id = &d[pos..pos + 4];
        let size = u32le(d, pos + 4)? as usize;
        let data_start = pos + 8;
        let data_end = data_start.checked_add(size)?;
        if data_end > d.len() {
            return None;
        }
        if id == b"icon" {
            return Some(&d[data_start..data_end]);
        }
        if id == b"LIST" && data_end - data_start >= 4 {
            // Pula o tipo da lista e varre as subchunks.
            if let Some(found) = find_icon_chunk(&d[data_start + 4..data_end]) {
                return Some(found);
            }
        }
        pos = data_start + size + (size & 1); // chunks alinhados em 2 bytes
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_cur(and_first: u8) -> Vec<u8> {
        let mut d = vec![0u8; 6 + 16];
        d[2] = 2; // type cursor
        d[4] = 1; // count
        d[6] = 1; // w
        d[7] = 1; // h
        d[12] = 32; // bitcount (offset 6+6)
        d[14] = 52; // bytesinres = 40+4+4+4? header40+xor4+and4 = 48
        d[14] = 48;
        d[18] = 22; // offset = 6+16 = 22
        d.extend_from_slice(&40u32.to_le_bytes()); // biSize
        d.extend_from_slice(&1i32.to_le_bytes()); // w
        d.extend_from_slice(&2i32.to_le_bytes()); // h (xor+and)
        d.extend_from_slice(&1u16.to_le_bytes()); // planes
        d.extend_from_slice(&32u16.to_le_bytes()); // bpp
        d.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        d.extend_from_slice(&[0u8; 20]); // resto do header (40-20=20 ja? header tem 40: 4+4+4+2+2+4+4+4+4+4+4 = 40; escrevemos 4+4+4+2+2+4=20, faltam 20)
        d.extend_from_slice(&[0, 0, 255, 255]); // pixel BGRA = vermelho opaco
        d.extend_from_slice(&[and_first, 0, 0, 0]); // AND mask
        d
    }

    #[test]
    fn parses_opaque_pixel() {
        let img = parse_cur_bytes(&tiny_cur(0)).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.rgba, vec![255, 0, 0, 255]);
    }

    #[test]
    fn and_mask_clears_alpha() {
        let img = parse_cur_bytes(&tiny_cur(0x80)).unwrap();
        assert_eq!(img.rgba, vec![255, 0, 0, 0]);
    }

    #[test]
    fn rejects_non_cursor() {
        assert!(parse_cur_bytes(b"nao e cursor........").is_none());
    }

    fn tiny_icon_image() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&1i32.to_le_bytes());
        v.extend_from_slice(&2i32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&32u16.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&[0u8; 20]);
        v.extend_from_slice(&[0, 0, 255, 255]);
        v.extend_from_slice(&[0x80, 0, 0, 0]); // AND: transparente
        v
    }

    #[test]
    fn ani_first_frame() {
        let icon = tiny_icon_image();
        // Chunk 'icon' real: ICONDIR + 1 entrada + imagem.
        let mut chunk = vec![0u8; 6 + 16];
        chunk[2] = 1; // icone
        chunk[4] = 1; // count
        chunk[6] = 1; // w
        chunk[7] = 1; // h
        chunk[12] = 32; // bpp
        chunk[14..18].copy_from_slice(&(icon.len() as u32).to_le_bytes());
        chunk[18..22].copy_from_slice(&22u32.to_le_bytes());
        chunk.extend_from_slice(&icon);
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&0u32.to_le_bytes()); // tamanho recalculado abaixo
        d.extend_from_slice(b"ACON");
        d.extend_from_slice(b"anih");
        d.extend_from_slice(&36u32.to_le_bytes());
        d.extend_from_slice(&[0u8; 36]);
        d.extend_from_slice(b"LIST");
        d.extend_from_slice(&(4 + 8 + chunk.len() as u32).to_le_bytes());
        d.extend_from_slice(b"fram");
        d.extend_from_slice(b"icon");
        d.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        d.extend_from_slice(&chunk);
        let total = (d.len() - 8) as u32;
        d[4..8].copy_from_slice(&total.to_le_bytes());
        let img = parse_ani_first_frame(&d).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.rgba, vec![255, 0, 0, 0]);
    }

    #[test]
    fn ani_rejects_junk() {
        assert!(parse_ani_first_frame(b"RIFF....JUNK....").is_none());
    }

    #[test]
    fn decodes_png_entry() {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        // Monta um .cur minimo apontando para o PNG.
        let mut d = vec![0u8; 6 + 16];
        d[2] = 2;
        d[4] = 1;
        d[6] = 32;
        d[7] = 32;
        d[12] = 32;
        d[14..18].copy_from_slice(&(png.len() as u32).to_le_bytes());
        d[18..22].copy_from_slice(&22u32.to_le_bytes());
        d.extend_from_slice(&png);
        let img = parse_cur_bytes(&d).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.rgba, vec![255, 0, 0, 255]);
    }
}
