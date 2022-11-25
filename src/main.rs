use clap::{Arg, Command};
use regex::bytes::RegexBuilder;
use std::{
    fs::File,
    io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom},
    process::exit,
};

fn search_regex(file: &File, pattern: &str) -> Result<u64, io::Error> {
    let mut buff = BufReader::new(file);
    let mut bytes = vec![0; 1024];
    // Disable Unicode (\u flag) to search arbitrary (non-UTF-8) bytes
    let re = if let Ok(re) = RegexBuilder::new(pattern).unicode(false).build() {
        re
    } else {
        return Err(io::Error::from(ErrorKind::InvalidInput));
    };

    buff.seek(SeekFrom::Start(0))?;
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
                    return Ok(buff.stream_position().unwrap() - (read - m.start()) as u64);
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

    Err(io::Error::from(ErrorKind::NotFound))
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
        let mut bytes = bytes[2..].to_string();
        // prefix a '0' if the len isn't odd
        if bytes.len() % 2 != 0 {
            bytes.insert(0, '0');
        }
        assert!(bytes.len() % 2 == 0);
        match bytes.len() {
            2 => {
                // a single byte, endianness doesn't matter
                pattern = String::from(bytes);
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
            pattern += &(String::from(r"\x") + byte)
        });
    }

    let file = matches.get_one::<String>("file").unwrap();
    let file = match File::open(file) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("Failed to open file {file}: {err}");
            return;
        }
    };

    if let Ok(offset) = search_regex(&file, &pattern) {
        println!("{offset}");
    } else {
        eprintln!("Cannot find the bytes: {bytes}");
    }
}
