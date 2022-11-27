use clap::{value_parser, Arg, Command};
use regex::bytes::RegexBuilder;
use std::{
    fs::File,
    io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom},
    ops::Range,
    process::exit,
};

const G_VT_DEFAULT: &str = "\x1B[0m";
const G_VT_RED: &str = "\x1B[91m";
const G_LINE_WIDTH: usize = 16;

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
                if let Some(m) = re.find(&bytes[..read]) {
                    offsets.extend_from_slice(&[
                        buff.stream_position().unwrap() as usize - (read - m.start())
                    ]);
                } else {
                    // overlap the search around the chunk boundaries
                    // in case the pattern locates across the boundary
                    buff.seek(SeekFrom::Current(1 - pattern.len() as i64))?;
                }
            }
            Err(err) => {
                return Err(err);
            }
        }
    }

    Ok(offsets)
}

fn read_and_print_one_line(file: &mut File, line_offset: usize, range: Range<usize>) {
    let mut bytes = vec![0; G_LINE_WIDTH];
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
        if i % (G_LINE_WIDTH / 2) == 0 {
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
                print!("{}", *byte as char,);
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
        .about("A utility to search arbitrary bytes in a file")
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
            Arg::new("bytes")
                .help("Quoted bytes in hexadecimal format either without 0x (e.g.: \"1f 8b 08\")\nor with 0x in one word and respect --endian argument (e.g.: -e little 0x088b1f)")
                .required(true),
        )
        .arg(Arg::new("file").help("file to search").required(true))
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
    let file = matches.get_one::<String>("file").unwrap();
    let mut file = match File::open(file) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("Failed to open file {file}: {err}");
            return;
        }
    };

    let filelen = file.metadata().unwrap().len();

    if let Ok(offsets) = search_regex(&file, &pattern) {
        let context = matches.get_one::<u8>("context").unwrap_or(&0);

        offsets.iter().for_each(|offset| {
            println!("offset: {offset} ({offset:08x})");
            let line_offset = offset - offset % G_LINE_WIDTH;

            // print before-context lines
            for i in (1..=*context).step_by(1).rev() {
                if line_offset < G_LINE_WIDTH * i as usize {
                    continue;
                }
                read_and_print_one_line(
                    &mut file,
                    line_offset - G_LINE_WIDTH * i as usize,
                    Range::default(),
                );
            }

            // each byte in pattern is represented as 4 characters, e.g.: "\xAA"
            let bytes = pattern.len() / 4;
            let byte_offset_start = offset % G_LINE_WIDTH;
            // byte_offset_end is the offset of ending color byte (exclusive) in its own line,
            // which might be different from the line of byte_offset_start
            let byte_offset_end = (byte_offset_start + bytes) % G_LINE_WIDTH;
            // when pattern ends at the end of the line, set the byte_offset_end to line width
            // so that printing function can work properly
            let byte_offset_end = if byte_offset_end == 0 {
                G_LINE_WIDTH
            } else {
                byte_offset_end
            };

            // calculate how many lines the pattern overlaps
            let color_lines = {
                // not start at the line beginning and overlap the line ending
                let (start_line, remaining_bytes) = if byte_offset_start % G_LINE_WIDTH != 0
                    && byte_offset_start + bytes > G_LINE_WIDTH
                {
                    (1, bytes - (G_LINE_WIDTH - byte_offset_start))
                } else {
                    (0, bytes)
                };

                start_line + (remaining_bytes + G_LINE_WIDTH - 1) / G_LINE_WIDTH
            };
            // print color lines
            for i in (0..color_lines).step_by(1) {
                read_and_print_one_line(
                    &mut file,
                    line_offset + G_LINE_WIDTH * i,
                    Range {
                        start: if i == 0 { byte_offset_start } else { 0 },
                        end: if i == color_lines - 1 {
                            byte_offset_end
                        } else {
                            G_LINE_WIDTH
                        },
                    },
                )
            }

            // move line_offset pointing to next line of color lines
            let line_offset = line_offset + G_LINE_WIDTH * color_lines;
            // print after-context lines
            for i in (0..*context).step_by(1) {
                // only check the start offset of the line
                // and let read_and_print_one_line() handle the end offset of this line
                if line_offset + G_LINE_WIDTH * i as usize >= filelen as usize {
                    println!("(EOF)");
                    break;
                }
                read_and_print_one_line(
                    &mut file,
                    line_offset + G_LINE_WIDTH * i as usize,
                    Range::default(),
                );
            }

            println!();
        });
    } else {
        eprintln!("Cannot find the bytes: {bytes}");
    }
}
