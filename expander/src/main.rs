extern crate rustfmt;
extern crate syntex_syntax as syntax;

use syntax::ast;
use syntax::ext::base::{ExtCtxt, SyntaxExtension};
use syntax::ext::expand::{self, ExpansionConfig};
use syntax::codemap::CodeMap;
use syntax::errors::Handler;
use syntax::errors::emitter::{ColorConfig};
use syntax::fold::Folder;
use syntax::parse::{self, ParseSess};
use syntax::parse::token::intern;
use syntax::ptr::P;

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
    let expanded = expand::expand_crate(ecx, Vec::new(), Vec::new(),
                                                     data.krates[data.index].clone()).0;

    data.krates.push(expanded);
    data.index += 1; //Next expansion/write should happen on new crate
}


struct OnceExpander <'a, 'b:'a> {
    pub cx: &'a mut ExtCtxt<'b>,
}

impl<'a, 'b> OnceExpander<'a, 'b> {
    pub fn new(cx: &'a mut ExtCtxt<'b>) -> OnceExpander<'a, 'b> {
        OnceExpander { cx: cx }
    }
}
//TODO: Changed fold_expr, fold_pat, fold_stmt & fold_ty
//TODO: (Still need to look at Blocks & Items)
impl<'a, 'b> Folder for OnceExpander<'a, 'b> {
    fn fold_crate(&mut self, c: Crate) -> Crate {
        self.cx.filename = Some(self.cx.parse_sess.codemap().span_to_filename(c.span));
        expand::noop_fold_crate(c, self)
    }

    fn fold_expr(&mut self, expr: P<ast::Expr>) -> P<ast::Expr> {
        expand::expand_expr(expr, self, |_, expr| expr)
    }

    fn fold_pat(&mut self, pat: P<ast::Pat>) -> P<ast::Pat> {
        expand::expand_pat(pat, self, |_, p| p)
    }

    fn fold_item(&mut self, item: P<ast::Item>) -> SmallVector<P<ast::Item>> {
        use std::mem::replace;
        let result;
        if let ast::ItemKind::Mod(ast::Mod { inner, .. }) = item.node {
            if item.span.contains(inner) {
                self.push_mod_path(item.ident, &item.attrs);
                result = expand::expand_item(item, self);
                self.pop_mod_path();
            } else {
                let filename = if inner != codemap::DUMMY_SP {
                    Some(self.cx.parse_sess.codemap().span_to_filename(inner))
                } else { None };
                let orig_filename = expand::replace(&mut self.cx.filename, filename);
                let orig_mod_path_stack = expand::replace(&mut self.cx.mod_path_stack, Vec::new());
                result = expand::expand_item(item, self);
                self.cx.filename = orig_filename;
                self.cx.mod_path_stack = orig_mod_path_stack;
            }
        } else {
            result = expand::expand_item(item, self);
        }
        result
    }

    fn fold_item_kind(&mut self, item: ast::ItemKind) -> ast::ItemKind {
        expand::expand_item_kind(item, self)
    }

    fn fold_stmt(&mut self, stmt: ast::Stmt) -> SmallVector<ast::Stmt> {
        expand::expand_stmt(stmt, self, |_, s| s)
    }

    fn fold_block(&mut self, block: P<Block>) -> P<Block> {
        let was_in_block = ::std::mem::replace(&mut self.cx.in_block, true);
        let result = expand::expand_block(block, self);
        self.cx.in_block = was_in_block;
        result
    }

    fn fold_arm(&mut self, arm: ast::Arm) -> ast::Arm {
        expand::expand_arm(arm, self)
    }

    fn fold_trait_item(&mut self, i: ast::TraitItem) -> SmallVector<ast::TraitItem> {
        expand::expand_annotatable(Annotatable::TraitItem(P(i)), self)
            .into_iter().map(|i| i.expect_trait_item()).collect()
    }

    fn fold_impl_item(&mut self, i: ast::ImplItem) -> SmallVector<ast::ImplItem> {
        expand::expand_annotatable(Annotatable::ImplItem(P(i)), self)
            .into_iter().map(|i| i.expect_impl_item()).collect()
    }

    fn fold_ty(&mut self, ty: P<ast::Ty>) -> P<ast::Ty> {
        expand::expand_type(ty, self, |_, t| t)
    }

    fn new_span(&mut self, span: Span) -> Span {
        expand::new_span(self.cx, span)
    }
}

impl<'a, 'b> OnceExpander<'a, 'b> {
    fn push_mod_path(&mut self, id: Ident, attrs: &[ast::Attribute]) {
        let default_path = id.name.as_str();
        let file_path = match ::attr::first_attr_value_str_by_name(attrs, "path") {
            Some(d) => d,
            None => default_path,
        };
        self.cx.mod_path_stack.push(file_path)
    }

    fn pop_mod_path(&mut self) {
        self.cx.mod_path_stack.pop().unwrap();
    }
}
