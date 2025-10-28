#[allow(non_snake_case)]
pub fn FOURCC(code: &str) -> u32 {
    let mut chars = code.bytes();
    //let count = chars.count();

    let a0 = chars.next().unwrap() as u32;
    let a1 = chars.next().unwrap() as u32;
    let a2 = chars.next().unwrap() as u32;
    let a3 = chars.next().unwrap() as u32;

    return (a3) | ((a2) << 8) | ((a1) << 16) | ((a0) << 24);
}

pub fn fourcc_to_string(fourcc: u32) -> String {
    let chars = vec![
        ((fourcc >> 24) & 0xFF) as u8,
        ((fourcc >> 16) & 0xFF) as u8,
        ((fourcc >> 8) & 0xFF) as u8,
        ((fourcc) & 0xFF) as u8,
    ];
    return String::from_utf8(chars).unwrap();
}

pub fn human_version(ver: u16) -> u16 {
    // This is based on Lingo's `the fileVersion` with a correction to the
    // version number for Director 12.
    if ver >= 1951 {
        return 1200;
    }
    if ver >= 1922 {
        return 1150;
    }
    if ver >= 1921 {
        return 1100;
    }
    if ver >= 1851 {
        return 1000;
    }
    if ver >= 1700 {
        return 850;
    }
    if ver >= 1410 {
        return 800;
    }
    if ver >= 1224 {
        return 700;
    }
    if ver >= 1218 {
        return 600;
    }
    if ver >= 1201 {
        return 500;
    }
    if ver >= 1117 {
        return 404;
    }
    if ver >= 1115 {
        return 400;
    }
    if ver >= 1029 {
        return 310;
    }
    if ver >= 1028 {
        return 300;
    }
    return 200;
}
