use clap::{value_parser, Arg, Command};
use regex::bytes::RegexBuilder;
use std::{
    fs::File,
    io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom},
    ops::Range,
    process::exit,
};

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

    const G_VT_DEFAULT: &str = "\x1B[0m";
    const G_VT_RED: &str = "\x1B[91m";
    const G_LINE_WIDTH: usize = 16;

    let filelen = file.metadata().unwrap().len();

    let read_and_print_one_line = |file: &mut File, line_offset: usize, range: Range<usize>| {
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
    };

    if let Ok(offsets) = search_regex(&file, &pattern) {
        let context = matches.get_one::<u8>("context").unwrap_or(&0);

        offsets.iter().for_each(|offset| {
            println!("offset: {offset} ({offset:08x})");
            let line_offset = offset - offset % G_LINE_WIDTH;

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

            let start = offset - line_offset;
            // each byte in pattern is represented as 4 characters, e.g.: "\xAA"
            let end = start + pattern.len() / 4;
            read_and_print_one_line(&mut file, line_offset, Range { start, end });

            let mut plus = 0;
            // TODO: calculate how many lines the pattern overlaps
            // print one more line if the pattern overlaps the line boundary
            if end > G_LINE_WIDTH {
                plus = 1;
                read_and_print_one_line(
                    &mut file,
                    line_offset + G_LINE_WIDTH,
                    Range {
                        start: 0,
                        end: end - G_LINE_WIDTH,
                    },
                );
            }

            for i in (1 + plus..=*context + plus).step_by(1) {
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
