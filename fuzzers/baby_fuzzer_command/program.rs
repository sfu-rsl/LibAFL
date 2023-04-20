use std::io::Read;

fn main() {
    let mut buf = [0_u8; 100];
    std::io::stdin().read(&mut buf).unwrap();
    if !buf.is_empty() && buf[0] == b'a' {
        if buf.len() > 1 && buf[1] == b'b' {
            if buf.len() > 2 && buf[2] == b'c' {
                panic!("Artificial bug triggered =)");
            }
        }
    }
}
