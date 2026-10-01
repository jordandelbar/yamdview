//! Command line arguments.

use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, PartialEq)]
pub struct Args {
    pub print: bool,
    pub mouse: bool,
    pub images: Option<bool>,
    pub path: Option<PathBuf>,
    pub read_stdin: bool,
}

impl Args {
    pub fn new() -> Self {
        Args {
            print: false,
            mouse: true,
            images: None,
            path: None,
            read_stdin: false,
        }
    }
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut a = Args::new();
    let mut positional = false;
    for arg in args {
        let opt = (!positional).then(|| arg.to_string_lossy());
        let free = a.path.is_none() && !a.read_stdin;
        match opt.as_deref() {
            Some("--") => positional = true,
            Some("--mouse") => a.mouse = true,
            Some("--no-mouse") => a.mouse = false,
            Some("--images") => a.images = Some(true),
            Some("--no-images") => a.images = Some(false),
            Some("-p" | "--print") => a.print = true,
            Some("-") if free => a.read_stdin = true,
            Some(o) if o.starts_with('-') && o != "-" => {
                return Err(format!("unknown option: {o}"));
            }
            _ if free => a.path = Some(PathBuf::from(&arg)),
            _ => {
                return Err(
                    "usage: yamdview [-p|--print] [--mouse|--no-mouse] [--images|--no-images] [--] [FILE | -]"
                        .into(),
                );
            }
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_args() {
        let parse = |args: &[&str]| parse_args(args.iter().map(OsString::from));
        let a = parse(&["-p", "--no-mouse", "--images", "doc.md"]).unwrap();
        assert!(a.print && !a.mouse && a.images == Some(true));
        assert_eq!(a.path, Some(PathBuf::from("doc.md")));
        assert!(parse(&["-"]).unwrap().read_stdin);
        let a = parse(&["--", "-"]).unwrap();
        assert!(
            !a.read_stdin && a.path == Some(PathBuf::from("-")),
            "`-` after `--` is a file"
        );
        assert_eq!(
            parse(&["--", "--print"]).unwrap().path,
            Some(PathBuf::from("--print"))
        );
        assert_eq!(parse(&["-x"]).unwrap_err(), "unknown option: -x");
        assert!(parse(&["a.md", "b.md"]).unwrap_err().starts_with("usage"));
        assert!(parse(&["a.md", "-"]).unwrap_err().starts_with("usage"));
    }
}
