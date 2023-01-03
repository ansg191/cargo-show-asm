use std::borrow::Cow;
use std::path::Path;

use nom::branch::alt;
use nom::bytes::complete::{tag, take_while1, take_while_m_n};
use nom::character::complete;
use nom::character::complete::{newline, not_line_ending, space1};
use nom::combinator::{map, opt, recognize, verify};
use nom::sequence::{delimited, pair, preceded, terminated, tuple};
use nom::{AsChar, IResult};
use owo_colors::OwoColorize;

use crate::demangle::LabelKind;
use crate::{color, demangle};

#[derive(Clone, Debug)]
pub enum Statement<'a> {
    Label(Label<'a>),
    Directive(Directive<'a>),
    Instruction(Instruction<'a>),
    Nothing,
    Dunno(&'a str),
}

#[derive(Clone, Debug)]
pub struct Instruction<'a> {
    pub op: &'a str,
    pub args: Option<&'a str>,
}

impl<'a> Instruction<'a> {
    pub fn parse(input: &'a str) -> IResult<&'a str, Self> {
        preceded(tag("\t"), alt((Self::parse_regular, Self::parse_sharp)))(input)
    }

    fn parse_sharp(input: &'a str) -> IResult<&'a str, Self> {
        let sharps = take_while_m_n(1, 2, |c| c == '#');
        let sharp_tag = pair(sharps, not_line_ending);
        map(recognize(sharp_tag), |op| Instruction { op, args: None })(input)
    }

    fn parse_regular(input: &'a str) -> IResult<&'a str, Self> {
        // NOTE: ARM allows `.` inside instruction names e.g. `b.ne` for branch not equal
        //       Wasm also uses `.` in instr names, and uses `_` for `end_function`
        let op = take_while1(|c| AsChar::is_alphanum(c) || matches!(c, '.' | '_'));
        let args = opt(preceded(space1, not_line_ending));
        map(pair(op, args), |(op, args)| Instruction { op, args })(input)
    }
}

impl std::fmt::Display for Instruction<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", color!(self.op, OwoColorize::bright_blue))?;
        if let Some(args) = self.args {
            let args = demangle::contents(args, f.alternate());
            write!(f, " {}", demangle::color_local_labels(&args))?;
        }
        Ok(())
    }
}

impl std::fmt::Display for Statement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Statement::Label(l) => l.fmt(f),
            Statement::Directive(d) => {
                if f.alternate() {
                    write!(f, "{d:#}")
                } else {
                    write!(f, "{d}")
                }
            }
            Statement::Instruction(i) => {
                if f.alternate() {
                    write!(f, "\t{i:#}")
                } else {
                    write!(f, "\t{i}")
                }
            }
            Statement::Nothing => Ok(()),
            Statement::Dunno(l) => write!(f, "{l}"),
        }
    }
}

impl std::fmt::Display for Directive<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Directive::File(ff) => ff.fmt(f),
            Directive::Loc(l) => l.fmt(f),
            Directive::Generic(g) => g.fmt(f),
            Directive::Set(g) => {
                f.write_str(&format!(".set {}", color!(g, OwoColorize::bright_black)))
            }
            Directive::SectionStart(s) => {
                let dem = demangle::contents(s, f.alternate());
                f.write_str(&format!(
                    "{} {}",
                    color!(".section", OwoColorize::bright_black),
                    dem
                ))
            }
            Directive::SubsectionsViaSym => f.write_str(&format!(
                ".{}",
                color!("subsections_via_symbols", OwoColorize::bright_black)
            )),
        }
    }
}

impl std::fmt::Display for FilePath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.as_full_path().display(), f)
    }
}

impl std::fmt::Display for File<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\t.file\t{} {}", self.index, self.path)?;
        if let Some(md5) = self.md5 {
            write!(f, " {md5}")?;
        }
        Ok(())
    }
}

impl std::fmt::Display for GenericDirective<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\t.{}",
            color!(
                demangle::contents(self.0, f.alternate()),
                OwoColorize::bright_black
            )
        )
    }
}

impl std::fmt::Display for Loc<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.extra {
            Some(x) => write!(
                f,
                "\t.loc\t{} {} {} {}",
                self.file, self.line, self.column, x
            ),
            None => write!(f, "\t.loc\t{} {} {}", self.file, self.line, self.column),
        }
    }
}

impl std::fmt::Display for Label<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:",
            color!(
                demangle::contents(self.id, f.alternate()),
                OwoColorize::bright_black
            )
        )
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Label<'a> {
    pub id: &'a str,
    pub kind: LabelKind,
}

impl<'a> Label<'a> {
    pub fn parse(input: &'a str) -> IResult<&'a str, Self> {
        // TODO: label can't start with a digit
        let regular = map(
            terminated(take_while1(good_for_label), tag(":")),
            |id: &str| Label {
                id,
                kind: demangle::label_kind(id),
            },
        );
        let quoted = map(
            delimited(tag("\""), take_while1(|c| c != '"'), tag("\":")),
            |id: &str| Label {
                id,
                kind: demangle::label_kind(id),
            },
        );
        alt((regular, quoted))(input)
    }
}

#[derive(Copy, Clone, Debug, Eq, Default)]
pub struct Loc<'a> {
    pub file: u64,
    pub line: u64,
    pub column: u64,
    pub extra: Option<&'a str>,
}

impl<'a> PartialEq for Loc<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.line == other.line
    }
}

impl<'a> Loc<'a> {
    pub fn parse(input: &'a str) -> IResult<&'a str, Self> {
        map(
            tuple((
                tag("\t.loc\t"),
                complete::u64,
                space1,
                complete::u64,
                space1,
                complete::u64,
                opt(preceded(tag(" "), take_while1(|c| c != '\n'))),
            )),
            |(_, file, _, line, _, column, extra)| Loc {
                file,
                line,
                column,
                extra,
            },
        )(input)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FilePath<'a> {
    FullPath(&'a str),
    PathAndFileName { path: &'a str, name: &'a str },
}

impl FilePath<'_> {
    pub fn as_full_path(&self) -> Cow<'_, Path> {
        match self {
            FilePath::FullPath(path) => Cow::Borrowed(Path::new(path)),
            FilePath::PathAndFileName { path, name } => Cow::Owned(Path::new(path).join(name)),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct File<'a> {
    pub index: u64,
    pub path: FilePath<'a>,
    pub md5: Option<&'a str>,
}

impl<'a> File<'a> {
    pub fn parse(input: &'a str) -> IResult<&'a str, Self> {
        fn filename(input: &str) -> IResult<&str, &str> {
            delimited(tag("\""), take_while1(|c| c != '"'), tag("\""))(input)
        }

        map(
            tuple((
                tag("\t.file\t"),
                complete::u64,
                space1,
                filename,
                opt(tuple((space1, filename))),
                opt(tuple((space1, complete::hex_digit1))),
            )),
            |(_, fileno, _, filepath, filename, md5)| File {
                index: fileno,
                path: match filename {
                    Some((_, filename)) => FilePath::PathAndFileName {
                        path: filepath,
                        name: filename,
                    },
                    None => FilePath::FullPath(filepath),
                },
                md5: md5.map(|(_, md5)| md5),
            },
        )(input)
    }
}

#[test]
fn test_parse_label() {
    assert_eq!(
        Label::parse("\"?dtor$3@?0?_ZN4rust4main17h90585feb19c01afdE@4HA\":"),
        Ok((
            "",
            Label {
                id: "?dtor$3@?0?_ZN4rust4main17h90585feb19c01afdE@4HA",
                kind: LabelKind::Global,
            }
        ))
    );

    assert_eq!(
        Label::parse("GCC_except_table0:"),
        Ok((
            "",
            Label {
                id: "GCC_except_table0",
                kind: LabelKind::Unknown,
            }
        ))
    );
    assert_eq!(
        Label::parse("__ZN4core3ptr50drop_in_place$LT$rand..rngs..thread..ThreadRng$GT$17hba90ed09529257ccE:"),
        Ok((
            "",
            Label {
                id: "__ZN4core3ptr50drop_in_place$LT$rand..rngs..thread..ThreadRng$GT$17hba90ed09529257ccE",
                kind: LabelKind::Global,
            }
        ))
    );
    assert_eq!(
        Label::parse(".Lexception0:"),
        Ok((
            "",
            Label {
                id: ".Lexception0",
                kind: LabelKind::Local
            }
        ))
    );
    assert_eq!(
        Label::parse("LBB0_1:"),
        Ok((
            "",
            Label {
                id: "LBB0_1",
                kind: LabelKind::Local
            }
        ))
    );
    assert_eq!(
        Label::parse("Ltmp12:"),
        Ok((
            "",
            Label {
                id: "Ltmp12",
                kind: LabelKind::Temp
            }
        ))
    );
}

#[test]
fn test_parse_loc() {
    assert_eq!(
        Loc::parse("\t.loc\t31 26 29"),
        Ok((
            "",
            Loc {
                file: 31,
                line: 26,
                column: 29,
                extra: None
            }
        ))
    );
    assert_eq!(
        Loc::parse("\t.loc\t31 26 29 is_stmt 0"),
        Ok((
            "",
            Loc {
                file: 31,
                line: 26,
                column: 29,
                extra: Some("is_stmt 0")
            }
        ))
    );
    assert_eq!(
        Loc::parse("\t.loc\t31 26 29 prologue_end"),
        Ok((
            "",
            Loc {
                file: 31,
                line: 26,
                column: 29,
                extra: Some("prologue_end")
            }
        ))
    );
}

#[test]
fn test_parse_file() {
    let (rest, file) = File::parse("\t.file\t9 \"/home/ubuntu/buf-test/src/main.rs\"").unwrap();
    assert!(rest.is_empty());
    assert_eq!(
        file,
        File {
            index: 9,
            path: FilePath::FullPath("/home/ubuntu/buf-test/src/main.rs"),
            md5: None
        }
    );
    assert_eq!(
        file.path.as_full_path(),
        Path::new("/home/ubuntu/buf-test/src/main.rs")
    );

    let (rest, file) = File::parse("\t.file\t9 \"/home/ubuntu/buf-test\" \"src/main.rs\"").unwrap();
    assert!(rest.is_empty());
    assert_eq!(
        file,
        File {
            index: 9,
            path: FilePath::PathAndFileName {
                path: "/home/ubuntu/buf-test",
                name: "src/main.rs"
            },
            md5: None,
        }
    );
    assert_eq!(
        file.path.as_full_path(),
        Path::new("/home/ubuntu/buf-test/src/main.rs")
    );

    let (rest, file) = File::parse(
        "\t.file\t9 \"/home/ubuntu/buf-test\" \"src/main.rs\" 74ab618651b843a815bf806bd6c50c19",
    )
    .unwrap();
    assert!(rest.is_empty());
    assert_eq!(
        file,
        File {
            index: 9,
            path: FilePath::PathAndFileName {
                path: "/home/ubuntu/buf-test",
                name: "src/main.rs"
            },
            md5: Some("74ab618651b843a815bf806bd6c50c19"),
        }
    );
    assert_eq!(
        file.path.as_full_path(),
        Path::new("/home/ubuntu/buf-test/src/main.rs")
    );
}

#[derive(Clone, Debug)]
pub enum Directive<'a> {
    File(File<'a>),
    Loc(Loc<'a>),
    Generic(GenericDirective<'a>),
    Set(&'a str),
    SubsectionsViaSym,
    SectionStart(&'a str),
}

#[derive(Clone, Debug)]
pub struct GenericDirective<'a>(pub &'a str);

pub fn parse_statement(input: &str) -> IResult<&str, Statement> {
    let label = map(Label::parse, Statement::Label);

    let file = map(File::parse, Directive::File);

    let loc = map(Loc::parse, Directive::Loc);

    let section = map(
        preceded(tag("\t.section"), take_while1(|c| c != '\n')),
        |s: &str| Directive::SectionStart(s.trim()),
    );
    let generic = map(preceded(tag("\t."), take_while1(|c| c != '\n')), |s| {
        Directive::Generic(GenericDirective(s))
    });
    let set = map(
        preceded(tag(".set"), take_while1(|c| c != '\n')),
        Directive::Set,
    );
    let ssvs = map(tag(".subsections_via_symbols"), |_| {
        Directive::SubsectionsViaSym
    });

    let dunno = map(take_while1(|c| c != '\n'), Statement::Dunno);
    // let dunno = |input: &str| todo!("{:?}", &input[..100]);

    let instr = map(Instruction::parse, Statement::Instruction);
    let nothing = map(verify(not_line_ending, str::is_empty), |_| {
        Statement::Nothing
    });

    let dir = map(
        alt((file, loc, set, ssvs, section, generic)),
        Statement::Directive,
    );

    // use terminated on the subparsers so that if the subparser doesn't consume the whole line, it's discarded
    // we assume that each label/instruction/directive will only take one line
    alt((
        terminated(label, newline),
        terminated(dir, newline),
        terminated(instr, newline),
        terminated(nothing, newline),
        terminated(dunno, newline),
    ))(input)
}

fn good_for_label(c: char) -> bool {
    c == '.'
        || c == '$'
        || c == '_'
        || ('a'..='z').contains(&c)
        || ('A'..='Z').contains(&c)
        || ('0'..='9').contains(&c)
}
impl Statement<'_> {
    pub(crate) fn is_end_of_fn(&self) -> bool {
        let check_id = |id: &str| id.strip_prefix('.').unwrap_or(id).starts_with("Lfunc_end");
        matches!(self, Statement::Label(Label { id, .. }) if check_id(id))
    }

    pub(crate) fn is_section_start(&self) -> bool {
        matches!(self, Statement::Directive(Directive::SectionStart(_)))
    }

    pub(crate) fn is_global(&self) -> bool {
        match self {
            Statement::Directive(Directive::Generic(GenericDirective(dir))) => {
                dir.starts_with("globl\t")
            }
            _ => false,
        }
    }
}
