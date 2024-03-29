use clap::{crate_version, value_parser, Arg, Command};
use regex::bytes::RegexBuilder;
use std::{
    fs::File,
    io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom},
    ops::Range,
    process::exit,
};

const G_VT_DEFAULT: &str = "\x1B[0m";
const G_VT_BOLD: &str = "\x1B[1m";
const G_VT_RED: &str = "\x1B[91m";

fn search_regex(file: &File, pattern: &str) -> Result<Vec<usize>, io::Error> {
    let mut buff = BufReader::new(file);
    let mut bytes = vec![0; 1024];
    // Disable Unicode (\u flag) to search arbitrary (non-UTF-8) bytes
    let re = if let Ok(re) = RegexBuilder::new(pattern).unicode(false).build() {
        re
    } else {
        return Err(io::Error::from(ErrorKind::InvalidInput));
    };

    buff.seek(SeekFrom::Start(0))?;
    let mut offsets = Vec::new();
    loop {
        match buff.read(&mut bytes) {
            Ok(read) => {
                if read == 0 {
                    break;
                }
                // Note: pattern.len() is the length of the string, not bytes
                if read < pattern.len() {
                    // if remaining bytes is shorter than a pattern,
                    // search again the last length of pattern
                    buff.seek(SeekFrom::End(pattern.len() as i64))?;
                    continue;
                }
                // find all non-overlapping matches
                for m in re.find_iter(&bytes[..read]) {
                    offsets.extend_from_slice(&[
                        buff.stream_position().unwrap() as usize - (read - m.start())
                    ]);
                }
                // overlap the search around the chunk boundaries
                // in case the pattern locates across the boundary
                buff.seek(SeekFrom::Current(1 - pattern.len() as i64))?;
            }
            Err(err) => {
                return Err(err);
            }
        }
    }

    if offsets.is_empty() {
        Err(io::Error::from(ErrorKind::NotFound))
    } else {
        Ok(offsets)
    }
}

fn read_and_print_one_line(
    file: &mut File,
    line_width: usize,
    line_offset: usize,
    range: Range<usize>,
) {
    let mut bytes = vec![0; line_width];
    if file.seek(SeekFrom::Start(line_offset as u64)).is_err() {
        return;
    }
    let read = file.read(&mut bytes[..]).unwrap_or_default();
    if read == 0 {
        return;
    }

    // header
    print!("{line_offset:08x}");

    // hexadecimal bytes
    for (i, byte) in bytes.iter().enumerate() {
        if line_width != 1 && i % (line_width / 2) == 0 {
            print!(" ");
        }
        if range.contains(&i) {
            print!("{G_VT_RED}");
        }
        if i < read {
            print!(" {byte:02x}");
        } else {
            // print spaces as place holder
            print!("   ");
        }
        print!("{G_VT_DEFAULT}");
    }

    // chracters
    print!("  |");
    for (i, byte) in bytes.iter().enumerate() {
        if range.contains(&i) {
            print!("{G_VT_RED}");
        }
        if i < read {
            if byte.is_ascii() && !byte.is_ascii_control() {
                print!("{}", *byte as char);
            } else {
                print!(".");
            }
        } else {
            print!(" ");
        }
        print!("{G_VT_DEFAULT}");
    }
    println!("|");
}

fn main() {
    let matches = Command::new(env!("CARGO_BIN_NAME"))
        .about("A CLI utility to search arbitrary bytes in files")
        .version(crate_version!())
        .arg_required_else_help(true)
        .arg(
            Arg::new("endian")
                .short('e')
                .long("endian")
                .help("Specify the endianness of the bytes")
                .value_parser(["big", "little"])
                .default_value("big"),
        )
        .arg(
            Arg::new("context")
                .short('c')
                .long("context")
                .value_parser(value_parser!(u8).range(1..=10))
                .help("Show 1-10 lines of context bytes when pattern is found in the file"),
        )
        .arg(
            Arg::new("width")
                .short('w')
                .long("width")
                .value_parser(value_parser!(u8).range(1..))
                .default_value("16")
                .help("Line width when printing the search result"),
        )
        .arg(
            Arg::new("bytes")
                .help("Quoted bytes in hexadecimal format either without 0x (e.g.: \"1f 8b 08\")\nor with 0x in one word and respect --endian argument (e.g.: -e little 0x088b1f)")
                .required(true),
        )
        .arg(Arg::new("files").help("files to search").required(true).num_args(1..))
        .get_matches();

    let mut pattern = String::new();
    let bytes = matches
        .get_one::<String>("bytes")
        .unwrap()
        .trim()
        .to_lowercase();

    let check_byte_or_exit = |byte| {
        if u8::from_str_radix(byte, 16).is_err() {
            eprintln!("{byte} isn't a hexadecimal byte.");
            exit(-1);
        }
    };

    // bytes in format "0x088b1f"
    if bytes.starts_with("0x") {
        // trim off "0x" first
        let mut bytes = bytes.strip_prefix("0x").unwrap().to_string();
        // prefix a '0' if the len isn't odd
        if bytes.len() % 2 != 0 {
            bytes.insert(0, '0');
        }
        assert!(bytes.len() % 2 == 0);
        match bytes.len() {
            2 => {
                // a single byte, endianness doesn't matter
                pattern = bytes;
            }
            _not_shorter_than_4 => {
                for i in (0..bytes.len()).step_by(2) {
                    let byte = &bytes[i..=i + 1];
                    check_byte_or_exit(byte);
                    // only need to swap bytes when it's litten-endian
                    if matches.get_one::<String>("endian").unwrap() == "little" {
                        pattern.insert_str(0, &(String::from(r"\x") + byte));
                    } else {
                        pattern += &(String::from(r"\x") + byte);
                    }
                }
            }
        }
    } else {
        // bytes in format "1f 8b 08"
        bytes.split_whitespace().for_each(|byte| {
            check_byte_or_exit(byte);
            // prefix a '0' if the len isn't 2 for regex matching
            // after checking the byte is a valid u8 hexadecimal,
            // it's safe to check the len is 1 or not.
            if byte.len() == 1 {
                pattern += &(String::from(r"\x0") + byte);
            } else {
                pattern += &(String::from(r"\x") + byte);
            }
        });
    }

    // TODO: add support of reading stdin
    let paths = matches.get_many::<String>("files").unwrap();
    paths.for_each(|path| {
        let mut file = match File::open(path) {
            Ok(image) => image,
            Err(err) => {
                eprintln!("Failed to open file {path}: {err}");
                return;
            }
        };

        let filelen = file.metadata().unwrap().len();

        println!("{G_VT_BOLD}{path}{G_VT_DEFAULT}:\n");
        if let Ok(offsets) = search_regex(&file, &pattern) {
            let context = matches.get_one::<u8>("context").unwrap_or(&0);
            // width argument has default value so it's safe to unwrap
            let line_width = *matches.get_one::<u8>("width").unwrap() as usize;

            offsets.iter().for_each(|offset| {
                println!("offset: {offset} ({offset:08x})");
                let line_offset = offset - offset % line_width;

                // print before-context lines
                for i in (1..=*context).step_by(1).rev() {
                    if line_offset < line_width * i as usize {
                        continue;
                    }
                    read_and_print_one_line(
                        &mut file,
                        line_width,
                        line_offset - line_width * i as usize,
                        Range::default(),
                    );
                }

                // each byte in pattern is represented as 4 characters, e.g.: "\xAA"
                let bytes = pattern.len() / 4;
                let byte_offset_start = offset % line_width;
                // byte_offset_end is the offset of ending color byte (exclusive) in its own line,
                // which might be different from the line of byte_offset_start
                let byte_offset_end = (byte_offset_start + bytes) % line_width;
                // when pattern ends at the end of the line, set the byte_offset_end to line width
                // so that printing function can work properly
                let byte_offset_end = if byte_offset_end == 0 {
                    line_width
                } else {
                    byte_offset_end
                };

                // calculate how many lines the pattern overlaps
                let color_lines = {
                    // not start at the line beginning and overlap the line ending
                    let (start_line, remaining_bytes) = if byte_offset_start % line_width != 0
                        && byte_offset_start + bytes > line_width
                    {
                        (1, bytes - (line_width - byte_offset_start))
                    } else {
                        (0, bytes)
                    };

                    start_line + (remaining_bytes + line_width - 1) / line_width
                };
                // print color lines
                for i in (0..color_lines).step_by(1) {
                    read_and_print_one_line(
                        &mut file,
                        line_width,
                        line_offset + line_width * i,
                        Range {
                            start: if i == 0 { byte_offset_start } else { 0 },
                            end: if i == color_lines - 1 {
                                byte_offset_end
                            } else {
                                line_width
                            },
                        },
                    )
                }

                // move line_offset pointing to next line of color lines
                let line_offset = line_offset + line_width * color_lines;
                // print after-context lines
                for i in (0..*context).step_by(1) {
                    // only check the start offset of the line
                    // and let read_and_print_one_line() handle the end offset of this line
                    if line_offset + line_width * i as usize >= filelen as usize {
                        println!("(EOF)");
                        break;
                    }
                    read_and_print_one_line(
                        &mut file,
                        line_width,
                        line_offset + line_width * i as usize,
                        Range::default(),
                    );
                }

                println!();
            });
        } else {
            eprintln!("Cannot find the bytes: {bytes}\n");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_regex() {
        let file = File::open("tests/data/vmlinuz-6.4-x86_64").unwrap();
        let offsets = search_regex(&file, r"\x1f\x8b\x08").unwrap();
        assert_eq!(offsets, vec![0x0061bd72, 0x006b7b9e, 0x0085ab9f]);
    }
}
