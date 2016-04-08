extern crate rustfmt;
extern crate syntex_syntax as syntax;

use syntax::ast;
use syntax::ext::base::{ExtCtxt, SyntaxExtension};
use syntax::ext::expand::ExpansionConfig;
use syntax::codemap::CodeMap;
use syntax::errors::Handler;
use syntax::errors::emitter::{ColorConfig};
use syntax::parse::{self, ParseSess};
use syntax::parse::token::intern;

use rustfmt::filemap::FileMap;
use rustfmt::config::{Config, WriteMode};
use rustfmt::modules::list_files;
use rustfmt::visitor::FmtVisitor;

use std::io::stdout;
use std::env;
use std::path::Path;
use std::rc::Rc;

struct ExpandData {
    crate_name: String,
    config: Config,
    session: ParseSess,
    krates: Vec<ast::Crate>,
    index: usize
}

fn main() {
    let mut data = init_data();
    write_file(&mut data);
    expand_crate(&mut data);
    write_file(&mut data);
}

fn init_data() -> ExpandData {
    let file = get_file();
    let mut config = Config::default();
    config.write_mode = WriteMode::Overwrite;
    let codemap = Rc::new(CodeMap::new());
    let tty_handler = Handler::with_tty_emitter(ColorConfig::Auto,
                                                None,
                                                true,
                                                false,
                                                codemap.clone());
    let session = ParseSess::with_span_handler(tty_handler, codemap.clone());
    let mut krates = vec!();
    krates.push(parse::parse_crate_from_file(&Path::new(&file), Vec::new(), &session).unwrap());
    ExpandData {
        crate_name: Path::new(&file).file_stem()
                                   .and_then(|stem| stem.to_str())
                                   .unwrap_or("").to_owned(),
        config: config,
        session: session,
        krates: krates,
        index: 0
    }
}



fn get_file() -> String {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        panic!("Insufficient number of arguments: Please pass in a filename to parse");
    }
    if args.len() > 3 {
        print!("Warning: Excess arguments ignored");
    }
    args[1].clone()
}

fn write_file(data: &mut ExpandData) {
    let mut fm = FileMap::new();
    for (path, module) in list_files(&data.krates[data.index], data.session.codemap()) {
        let path = path.to_str().unwrap();
        let mut visitor = FmtVisitor::from_codemap(&data.session, &data.config);
        visitor.format_separate_mod(module);
        fm.insert(path.to_owned(), visitor.buffer);
    }
    let out = &mut stdout();

    for filename in fm.keys() {
        let prefix = Path::new(filename).file_stem().and_then(|stem| stem.to_str()).unwrap_or("");
        let parent = Path::new(filename).parent().and_then(|path| path.to_str()).unwrap_or("");
        let file = format!("{}/{}Output{}.rs", parent, prefix, data.index);
        let res = rustfmt::filemap::write_file(&fm[filename], &file, out, &data.config);
        if let Err(_) = res {
            print!("Error writing to file {}", file);
        }
    }
}

fn expand_crate(data: &mut ExpandData) {
    let ex_cfg = ExpansionConfig::default(data.crate_name.clone());
    let mut tmp_vec = vec!();
    let mut ecx = ExtCtxt::new(&data.session,
                               data.krates[data.index].config.clone(),
                               ex_cfg,
                               &mut tmp_vec);
    ecx.syntax_env.insert(intern("macro_rules"), SyntaxExtension::MacroRulesTT);

    //TODO: Can't expand builtins yet
    //syntax_ext::register_builtins(&mut ecx.syntax_env);
    let expanded = syntax::ext::expand::expand_crate(ecx, Vec::new(), Vec::new(),
                                                     data.krates[data.index].clone()).0;

    data.krates.push(expanded);
    data.index += 1; //Next expansion/write should happen on new crate
}