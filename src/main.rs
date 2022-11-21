use clap::{Arg, Command};
use regex::bytes::RegexBuilder;
use std::{
    fs::File,
    io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom},
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
            Arg::new("bytes")
                .help("Quoted bytes in hexadecimal format without 0x (e.g.: \"1f 8b 08\")")
                .required(true),
        )
        .arg(Arg::new("file").help("file to search").required(true))
        .get_matches();

    let bytes = matches.get_one::<String>("bytes").unwrap();
    let split = bytes.split_whitespace();
    let mut pattern = String::new();
    split.for_each(|byte| pattern += &(String::from(r"\x") + byte));

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
    }
}
