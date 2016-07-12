extern crate rustfmt;
extern crate syntex_syntax as syntax;

use syntax::ast;
use syntax::ext::base::{ExtCtxt, DummyMacroLoader};
use syntax::ext::expand;
use syntax::ext::expand::{ExpansionConfig, MacroExpander};
use syntax::codemap::{CodeMap, Span, ExpnInfo, NO_EXPANSION};
use syntax::errors::Handler;
use syntax::errors::emitter::{ColorConfig};
use syntax::fold::{self, Folder};
use syntax::parse::{self, ParseSess};
use syntax::ptr::{self, P};
use syntax::util::small_vector::SmallVector;

use rustfmt::filemap::FileMap;
use rustfmt::config::{Config, WriteMode};
use rustfmt::modules::list_files;
use rustfmt::visitor::FmtVisitor;

use std::collections::HashMap;
use std::io::stdout;
use std::env;
use std::path::Path;
use std::rc::Rc;

// Small macro to simplify setting the full-expansion closures to the identity closure.
macro_rules! set_expander_fns {
    ($expander:ident,
        $( $expand:ident ),*) => {{
        $( $expander.$expand = Rc::new(Box::new(|_, node| node )); )*
    }}
}

struct ExpandData<'a> {
    config: Config,
    cx: ExtCtxt<'a>,
    krates: Vec<ast::Crate>,
    index: usize,
    span_map: HashMap<Span, Span>,
}

impl<'a> ExpandData<'a> {
    fn new(sess: &'a ParseSess, loader: &'a mut DummyMacroLoader) -> ExpandData<'a> {
        let args: Vec<String> = env::args().collect();
        if args.len() < 2 {
            panic!("Please supply a filepath to parse.")
        }
        if args.len() > 2 {
            panic!("Too many arguments. Please supply a single filepath.")
        }
        let filename = args[1].clone();

        let mut config = Config::default();
        config.write_mode = WriteMode::Overwrite;

        let ex_cfg = ExpansionConfig::default(filename.clone());
        let mut krates = vec!();
        krates.push(parse::parse_crate_from_file(&Path::new(&filename),
                                                 Vec::new(), sess).unwrap());
        let ecx = ExtCtxt::new(sess,
                               krates[0].config.clone(),
                               ex_cfg,
                               loader);
        ExpandData {
            config: config,
            cx: ecx,
            krates: krates,
            index: 0,
            span_map: HashMap::new(),
        }
    }

    fn insert(&mut self, span: Span) {
        if span.expn_id == NO_EXPANSION {
            return;
        }

        let key_sp = Span {
            lo: span.lo,
            hi: span.hi,
            expn_id: NO_EXPANSION
        };
        let callsite = self.cx.codemap().with_expn_info(span.expn_id,
                                                        |ei| ei.map(|ei| ei.call_site.clone()));
        if callsite.is_none() {
            panic!("Callsite not found!");
        }
        let mut callsite = callsite.unwrap();

        if !self.span_map.contains_key(&callsite) {
            self.span_map.insert(key_sp, span);
            return;
        }

        let callee = self.cx.codemap().with_expn_info(span.expn_id,
                                                      |ei| ei.map(|ei| ei.callee.clone()));
        if callee.is_none() {
            panic!("Callee not found!");
        }
        let callee = callee.unwrap();

        callsite = self.span_map.get(&callsite).unwrap().clone();
        let info = ExpnInfo {
            call_site: callsite,
            callee: callee
        };
        let new_id = self.cx.codemap().record_expansion(info);
        self.span_map.insert(key_sp, Span { expn_id: new_id, .. span });
    }

    fn get(&mut self, span: Span) -> Span {
        let key_sp = Span { expn_id: NO_EXPANSION, .. span };
        return self.span_map.get(&key_sp).unwrap_or(&span).clone();
    }

    fn expand_crate(&mut self) {
        let mut krate = self.krates[self.index].clone();
        {
            let mut expander = MacroExpander::new(&mut self.cx);

            set_expander_fns!(expander,
                                expand_pat,
                                expand_type,
                                expand_expr,
                                expand_stmt,
                                expand_item,
                                expand_impl_item,
                                expand_opt_expr);


            krate = expand::expand_crate(&mut expander,
                                         Vec::new(),
                                         krate).0;
        }

        krate = self.fold_crate(krate);

        self.krates.push(krate);
        self.index += 1;
    }

    fn write_file(&self) {
        let mut fm = FileMap::new();
        for (path, module) in list_files(&self.krates[self.index], self.cx.codemap()) {
            let path = path.to_str().unwrap();
            let mut visitor = FmtVisitor::from_codemap(self.cx.parse_sess, &self.config);
            visitor.format_separate_mod(module);
            fm.push((path.to_owned(), visitor.buffer));
        }
        let out = &mut stdout();

        for (ref filename, ref text) in fm {
            let prefix = Path::new(filename).file_stem().and_then(|stem| stem.to_str()).unwrap_or("");
            let parent = Path::new(filename).parent().and_then(|path| path.to_str()).unwrap_or("");
            let file = format!("{}/{}Output{}.rs", parent, prefix, self.index);
            let res = rustfmt::filemap::write_file(text, &file, out, &self.config);
            if let Err(_) = res {
                print!("Error writing to file {}", file);
            }
        }
    }
}

//Walk over AST of expanded crate to patch up spans
impl<'a> Folder for ExpandData<'a> {
    fn fold_pat(&mut self, pat: P<ast::Pat>) -> P<ast::Pat> {
        println!("Folding pat");
        if pat.span.expn_id == NO_EXPANSION {
            return fold::noop_fold_pat(pat, self);
        }
        
        self.insert(pat.span);
        fold::noop_fold_pat(pat.map(|elt| ast::Pat { span: self.get(elt.span), .. elt }), self)
    }

    fn fold_ty(&mut self, ty: P<ast::Ty>) -> P<ast::Ty> {
        println!("Folding ty");
        if ty.span.expn_id == NO_EXPANSION {
            return fold::noop_fold_ty(ty, self);
        }
        
        self.insert(ty.span);
        fold::noop_fold_ty(ty.map(|elt| ast::Ty { span: self.get(elt.span), .. elt }), self)
    }

    fn fold_expr(&mut self, expr: P<ast::Expr>) -> P<ast::Expr> {
        println!("Folding expr");
        if expr.span.expn_id == NO_EXPANSION {
            return ptr::P(fold::noop_fold_expr(expr.unwrap(), self));
        }
        
        self.insert(expr.span);
        ptr::P(fold::noop_fold_expr(expr.map(|elt| ast::Expr { span: self.get(elt.span), .. elt }).unwrap(), self))
    }

    fn fold_opt_expr(&mut self, opt: P<ast::Expr>) -> Option<P<ast::Expr>> {
        println!("Folding optexpr");
        if opt.span.expn_id == NO_EXPANSION {
            return fold::noop_fold_opt_expr(opt, self);
        }
        
        self.insert(opt.span);
        fold::noop_fold_opt_expr(opt.map(|elt| ast::Expr { span: self.get(elt.span), .. elt }), self)
    }

    fn fold_item(&mut self, item: P<ast::Item>) -> SmallVector<P<ast::Item>> {
        println!("Folding item");
        if item.span.expn_id == NO_EXPANSION {
            return fold::noop_fold_item(item, self);
        }
        
        self.insert(item.span);
        return fold::noop_fold_item(item.map(|elt| ast::Item { span: self.get(elt.span), .. elt }), self)
    }

    fn fold_stmt(&mut self, stmt: ast::Stmt) -> SmallVector<ast::Stmt> {
        println!("Folding statemet");
        if stmt.span.expn_id == NO_EXPANSION {
            return fold::noop_fold_stmt(stmt, self);
        }
        println!("Span: {}", self.cx.codemap().span_to_expanded_string(stmt.span.clone()));
        println!("Got Span: {}", self.cx.codemap().span_to_expanded_string(self.get(stmt.span.clone())));
        
        self.insert(stmt.span);
        return fold::noop_fold_stmt(ast::Stmt { span: self.get(stmt.span), .. stmt }, self)
    }

    fn fold_impl_item(&mut self, item: ast::ImplItem) -> SmallVector<ast::ImplItem> {
        println!("Folding implitem");
        if item.span.expn_id == NO_EXPANSION {
            return fold::noop_fold_impl_item(item, self);
        }
        
        self.insert(item.span);
        return fold::noop_fold_impl_item(ast::ImplItem { span: self.get(item.span), .. item }, self)
    }

    fn fold_mac(&mut self, mac: ast::Mac) -> ast::Mac {
        fold::noop_fold_mac(mac, self)
    }
}

// Struct for checking if expansion is required.
// (Checking if AST contains macros)
struct MacChecker {
    has_mac: bool,
}

impl MacChecker {

    fn new() -> MacChecker {
        MacChecker { has_mac: false }
    }

    fn check_finished(&mut self, data: &ExpandData) -> bool {
        self.has_mac = false;
        self.fold_crate(data.krates[data.index].clone());
        !self.has_mac
    }
}

impl Folder for MacChecker {
    fn fold_mac(&mut self, mac: ast::Mac) -> ast::Mac {
        self.has_mac = true;
        mac //No need to expand further
    }
}

// Given some filepath, repeatedly expand and write output until no further expansion possible
fn main() {
    let codemap = Rc::new(CodeMap::new());
    let tty_handler = Handler::with_tty_emitter(ColorConfig::Auto,
                                                None,
                                                true,
                                                false,
                                                codemap.clone());
    let session = ParseSess::with_span_handler(tty_handler, codemap.clone());
    let mut loader = DummyMacroLoader;
    let mut data = ExpandData::new(&session, &mut loader);
    data.write_file();
    while !MacChecker::new().check_finished(&data) {
        data.expand_crate();
        data.write_file();
    }
}