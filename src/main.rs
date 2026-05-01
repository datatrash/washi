use clap::{Parser, Subcommand};
use itertools::Itertools;
use shadow_rs::shadow;
use std::collections::{BTreeMap, BTreeSet};

pub mod format;

use crate::format::minify_wgsl_source;
use glob::glob;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use wgsl_parse::syntax::{
    CaseSelector, CompoundStatement, Expression, ExpressionNode, GlobalDeclaration, Ident,
    Statement, StatementNode, TranslationUnit, TypeExpression,
};
use wgsl_types::idents::{RESERVED_WORDS, iter_builtin_idents};

shadow!(build);

/// washi - A WGSL minifier
#[derive(Parser)]
#[clap(name = "washi", about, long_version = build::CLAP_LONG_VERSION, arg_required_else_help(true)
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Minify a WGSL shader
    Minify {
        /// The filename of the WGSL file to minify
        #[arg(value_hint = clap::ValueHint::FilePath)]
        input_filename: PathBuf,

        /// Output filename
        #[arg(value_hint = clap::ValueHint::FilePath)]
        output_filename: PathBuf,

        /// Generate map file (*.map)
        #[arg(short = 'm', long = "map")]
        generate_map: bool,
    },

    /// Minify multiple shaders at once, generating *.min.wgsl output files
    MinifyMultiple {
        #[arg(help = "a glob pattern, like 'src/**/*.wgsl'")]
        pattern: String,

        /// Generate map file (washi.map)
        #[arg(short = 'm', long = "map")]
        generate_map: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Minify {
            input_filename,
            output_filename,
            generate_map,
        } => {
            let mut minifier = Minifier::default();
            let mut module = TranslationUnit::from_str(&fs::read_to_string(&input_filename)?)?;
            minifier.minify(&mut module)?;

            fs::write(output_filename, minify_wgsl_source(&module.to_string()))?;
            if generate_map {
                minifier.write_map(&PathBuf::from(input_filename).with_extension("map"))?;
            }
        }
        Commands::MinifyMultiple {
            pattern,
            generate_map,
        } => {
            let mut minifier = Minifier::default();

            let files = glob(&pattern)?
                .filter_map(|r| match r {
                    Ok(r) => {
                        if r.to_string_lossy().contains(".min.") {
                            // Ignore already minified files
                            None
                        } else {
                            Some(r)
                        }
                    }
                    Err(_) => None,
                })
                .collect_vec();
            if files.is_empty() {
                return Ok(());
            }

            let mut result = BTreeMap::new();
            for file in &files {
                let mut module = TranslationUnit::from_str(&fs::read_to_string(&file)?)?;
                minifier.minify(&mut module)?;
                result.insert(file, minify_wgsl_source(&module.to_string()));
            }
            for (filename, minified) in result {
                let out = PathBuf::from(filename).with_extension("min.wgsl");
                fs::write(out, minified)?;
            }
            if generate_map {
                let map_path = find_rootmost(&files).expect("Could not determine map output path");
                minifier.write_map(&map_path.join("washi.map"))?;
            }
        }
    }

    Ok(())
}

// very VERY hacky way to deal with swizzles for now
pub const SWIZZLES: &[char] = &['r', 'g', 'b', 'a', 'x', 'y', 'z', 'w'];

#[derive(Default)]
pub struct Identifier(usize);

impl Identifier {
    pub fn next(&mut self) -> String {
        loop {
            let mut n = self.0;
            self.0 += 1;

            let mut identifier = String::new();
            let charset = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
            let base = charset.len();

            loop {
                let remainder = n % base;
                identifier.push(charset[remainder] as char);
                n /= base;

                if n == 0 {
                    break;
                }

                n -= 1;
            }

            let id = identifier.chars().rev().collect::<String>();
            if !Identifier::is_reserved(&id) {
                return id;
            }
        }
    }

    pub fn is_reserved(id: &str) -> bool {
        id.chars().all(|c| SWIZZLES.contains(&c))
            || RESERVED_WORDS.contains(&id)
            || iter_builtin_idents().contains(id)
    }

    pub fn next_ident(&mut self) -> Ident {
        Ident::new(self.next())
    }
}

#[derive(Default)]
pub struct Minifier {
    /// Generator for globally scoped renamed identifiers.
    global_id: Identifier,
    /// Original global identifier -> renamed global identifier mapping.
    global_ident_map: BTreeMap<String, Ident>,
    /// Set of all globally assigned names that locals must never reuse.
    global_taken_names: BTreeSet<String>,
    /// Per-function local name generator (reset when entering each function).
    local_id: Option<Identifier>,
    /// Stack of lexical local scopes used for declaration and lookup resolution.
    local_scopes: Vec<BTreeMap<String, Ident>>,
    /// Flattened mapping entries written to washi.map (globals only).
    map_entries: Vec<(String, String)>,
}

impl Minifier {
    pub fn minify(&mut self, module: &mut TranslationUnit) -> anyhow::Result<()> {
        // Pass 1: predeclare every module-scope identifier (struct names + members,
        // global declarations, type aliases, and non-entry-point function names) so
        // that `global_taken_names` is fully populated before any local rename can
        // run. Locals must never be allowed to alias any module-scope short name --
        // otherwise a function parameter could shadow a module-scope `var<private>`
        // that the body references, producing nonsense WGSL.
        for decl in &mut module.global_declarations {
            match decl.node_mut() {
                GlobalDeclaration::Struct(s) => {
                    self.map_global_ident(&mut s.ident);

                    for m in &mut s.members {
                        self.map_global_ident(&mut m.ident);
                    }
                }
                GlobalDeclaration::Declaration(d) => {
                    self.map_global_ident(&mut d.ident);
                }
                GlobalDeclaration::TypeAlias(a) => {
                    self.map_global_ident(&mut a.ident);
                }
                GlobalDeclaration::Function(f) => {
                    if f.attributes.is_empty() {
                        self.map_global_ident(&mut f.ident);
                    } else {
                        self.no_map_global(&mut f.ident);
                    }
                }
                _ => {}
            }
        }

        // Pass 2: minify global type expressions / initializers and function bodies.
        // By this point `global_taken_names` is complete, so `map_local_decl` can
        // safely skip every globally-used short name.
        for decl in &mut module.global_declarations {
            match decl.node_mut() {
                GlobalDeclaration::Struct(_s) => {}
                GlobalDeclaration::Declaration(d) => {
                    if let Some(t) = &mut d.ty {
                        self.minify_type_expression(t)?;
                    }
                    if let Some(expr) = &mut d.initializer {
                        self.minify_expr(expr)?;
                    }
                }
                GlobalDeclaration::TypeAlias(a) => {
                    self.minify_type_expression(&mut a.ty)?;
                }
                GlobalDeclaration::Function(f) => {
                    self.begin_function_scope();
                    for p in &mut f.parameters {
                        self.map_local_decl(&mut p.ident);
                        self.minify_type_expression(&mut p.ty)?;
                    }

                    self.minify_compound_statement(&mut f.body)?;

                    if let Some(t) = &mut f.return_type {
                        self.minify_type_expression(t)?;
                    }
                    self.end_function_scope();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn minify_stmt(&mut self, stmt: &mut StatementNode) -> anyhow::Result<()> {
        match stmt.node_mut() {
            Statement::Void => {}
            Statement::Compound(stmt) => {
                self.minify_compound_statement(stmt)?;
            }
            Statement::Assignment(stmt) => {
                self.minify_expr(&mut stmt.lhs)?;
                self.minify_expr(&mut stmt.rhs)?;
            }
            Statement::Increment(stmt) => {
                self.minify_expr(&mut stmt.expression)?;
            }
            Statement::Decrement(stmt) => {
                self.minify_expr(&mut stmt.expression)?;
            }
            Statement::If(stmt) => {
                self.minify_compound_statement(&mut stmt.if_clause.body)?;
                self.minify_expr(&mut stmt.if_clause.expression)?;
                if let Some(else_clause) = &mut stmt.else_clause {
                    self.minify_compound_statement(&mut else_clause.body)?;
                }
                for else_if in &mut stmt.else_if_clauses {
                    self.minify_compound_statement(&mut else_if.body)?;
                    self.minify_expr(&mut else_if.expression)?;
                }
            }
            Statement::Switch(stmt) => {
                self.minify_expr(&mut stmt.expression)?;
                for clause in &mut stmt.clauses {
                    self.minify_compound_statement(&mut clause.body)?;
                    for sel in &mut clause.case_selectors {
                        match sel {
                            CaseSelector::Default => {}
                            CaseSelector::Expression(expr) => {
                                self.minify_expr(expr)?;
                            }
                        }
                    }
                }
            }
            Statement::Loop(stmt) => {
                self.minify_compound_statement(&mut stmt.body)?;
            }
            Statement::For(stmt) => {
                self.push_local_scope();
                if let Some(stmt) = &mut stmt.initializer {
                    self.minify_stmt(stmt)?;
                }
                if let Some(expr) = &mut stmt.condition {
                    self.minify_expr(expr)?;
                }
                self.minify_compound_statement(&mut stmt.body)?;
                if let Some(stmt) = &mut stmt.update {
                    self.minify_stmt(stmt)?;
                }
                self.pop_local_scope();
            }
            Statement::While(stmt) => {
                self.minify_expr(&mut stmt.condition)?;
                self.minify_compound_statement(&mut stmt.body)?;
            }
            Statement::Break(_) => {}
            Statement::Continue(_) => {}
            Statement::Return(stmt) => {
                if let Some(expr) = &mut stmt.expression {
                    self.minify_expr(expr)?;
                }
            }
            Statement::Discard(_) => {}
            Statement::FunctionCall(stmt) => {
                self.minify_value_expression(&mut stmt.call.ty)?;
                for arg in &mut stmt.call.arguments {
                    self.minify_expr(arg)?;
                }
            }
            Statement::ConstAssert(stmt) => {
                self.minify_expr(&mut stmt.expression)?;
            }
            Statement::Declaration(stmt) => {
                if let Some(expr) = &mut stmt.initializer {
                    self.minify_expr(expr)?;
                }
                if let Some(t) = &mut stmt.ty {
                    self.minify_type_expression(t)?;
                }
                self.map_local_decl(&mut stmt.ident);
            }
        }

        Ok(())
    }

    fn minify_expr(&mut self, expr: &mut ExpressionNode) -> anyhow::Result<()> {
        match expr.node_mut() {
            Expression::Literal(_) => {}
            Expression::Parenthesized(expr) => {
                self.minify_expr(&mut expr.expression)?;
            }
            Expression::NamedComponent(expr) => {
                self.minify_expr(&mut expr.base)?;
                self.map_global_ident(&mut expr.component);
            }
            Expression::Indexing(expr) => {
                self.minify_expr(&mut expr.base)?;
                self.minify_expr(&mut expr.index)?;
            }
            Expression::Unary(expr) => {
                self.minify_expr(&mut expr.operand)?;
            }
            Expression::Binary(expr) => {
                self.minify_expr(&mut expr.left)?;
                self.minify_expr(&mut expr.right)?;
            }
            Expression::FunctionCall(expr) => {
                for arg in &mut expr.arguments {
                    self.minify_expr(arg)?;
                }
                self.minify_value_expression(&mut expr.ty)?;
            }
            Expression::TypeOrIdentifier(expr) => {
                self.minify_value_expression(expr)?;
            }
        }

        Ok(())
    }

    fn map_global_ident(&mut self, ident: &mut Ident) {
        let old = ident.to_string();

        if Identifier::is_reserved(&old) || is_preserved_ident(&old) {
            self.no_map_global(ident);
            return;
        }

        if self.global_ident_map.contains_key(&old) {
            *ident = self.global_ident_map.get(&old).unwrap().clone();
            self.global_taken_names.insert(ident.to_string());
            return;
        }

        // If a new global name is allocated lazily during function-body traversal,
        // also skip any names already issued to locals in the current function so we
        // don't introduce a backwards-shadowing collision.
        let new_ident = loop {
            let candidate = self.global_id.next();
            if !self.local_name_in_use(&candidate) {
                break Ident::new(candidate);
            }
        };
        self.global_ident_map.insert(old.clone(), new_ident.clone());
        self.global_taken_names.insert(new_ident.to_string());
        self.record_mapping(old, new_ident.to_string());
        *ident = new_ident;
    }

    fn no_map_global(&mut self, ident: &mut Ident) {
        let mapped = self
            .global_ident_map
            .entry(ident.to_string())
            .or_insert_with(|| ident.clone());
        self.global_taken_names.insert(mapped.to_string());
    }

    fn begin_function_scope(&mut self) {
        self.local_id = Some(Identifier::default());
        self.local_scopes.clear();
        self.push_local_scope();
    }

    fn end_function_scope(&mut self) {
        self.local_scopes.clear();
        self.local_id = None;
    }

    fn push_local_scope(&mut self) {
        if self.local_id.is_some() {
            self.local_scopes.push(BTreeMap::new());
        }
    }

    fn pop_local_scope(&mut self) {
        if self.local_id.is_some() {
            self.local_scopes.pop();
        }
    }

    fn map_local_decl(&mut self, ident: &mut Ident) {
        if self.local_id.is_none() {
            self.map_global_ident(ident);
            return;
        }

        let old = ident.to_string();
        if Identifier::is_reserved(&old) || is_preserved_ident(&old) {
            if let Some(scope) = self.local_scopes.last_mut() {
                scope.insert(old, ident.clone());
            }
            return;
        }

        let new_ident = loop {
            let candidate = self.local_id.as_mut().unwrap().next();
            if !self.global_taken_names.contains(&candidate) {
                break Ident::new(candidate);
            }
        };
        if let Some(scope) = self.local_scopes.last_mut() {
            scope.insert(old.clone(), new_ident.clone());
        }
        *ident = new_ident;
    }

    fn map_value_ident(&mut self, ident: &mut Ident) {
        let old = ident.to_string();
        if Identifier::is_reserved(&old) || is_preserved_ident(&old) {
            self.no_map_global(ident);
            return;
        }

        for scope in self.local_scopes.iter().rev() {
            if let Some(mapped) = scope.get(&old) {
                *ident = mapped.clone();
                return;
            }
        }

        self.map_global_ident(ident);
    }

    /// Returns true if any active local scope has already issued `name` as a renamed
    /// local. Used to keep lazy global rename allocations from colliding with locals
    /// already chosen for the function currently being minified.
    fn local_name_in_use(&self, name: &str) -> bool {
        self.local_scopes
            .iter()
            .any(|scope| scope.values().any(|ident| ident.to_string() == name))
    }

    fn record_mapping(&mut self, old: String, new: String) {
        self.map_entries.push((old, new));
    }

    fn minify_type_expression(&mut self, t: &mut TypeExpression) -> anyhow::Result<()> {
        match &mut t.template_args {
            Some(templ) => {
                for t in templ {
                    self.minify_expr(&mut t.expression)?;
                }
            }
            None => {
                self.map_global_ident(&mut t.ident);
            }
        }

        Ok(())
    }

    fn minify_value_expression(&mut self, t: &mut TypeExpression) -> anyhow::Result<()> {
        match &mut t.template_args {
            Some(templ) => {
                for t in templ {
                    self.minify_expr(&mut t.expression)?;
                }
            }
            None => {
                self.map_value_ident(&mut t.ident);
            }
        }

        Ok(())
    }

    fn minify_compound_statement(&mut self, stmt: &mut CompoundStatement) -> anyhow::Result<()> {
        self.push_local_scope();
        for stmt in &mut stmt.statements {
            self.minify_stmt(stmt)?;
        }
        self.pop_local_scope();
        Ok(())
    }

    pub fn write_map(&self, map: &Path) -> anyhow::Result<()> {
        let mut result = String::new();
        let mut seen = BTreeSet::new();
        for (old, new) in &self.map_entries {
            if old != new && seen.insert((old.clone(), new.clone())) {
                result += format!("{},{}\r\n", old, new).as_str();
            }
        }
        fs::write(&map, result)?;
        Ok(())
    }
}

fn is_preserved_ident(name: &str) -> bool {
    name.starts_with('_')
}

fn find_rootmost(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }

    let paths = paths.iter()
        .filter_map(|p| p.parent())
        .sorted_by_key(|p| p.components().count()).collect::<Vec<_>>();
    Some(paths[0].to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_data_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test-data")
            .join(name)
    }

    fn read_test_data(name: &str) -> anyhow::Result<String> {
        Ok(fs::read_to_string(test_data_path(name))?)
    }

    fn minify_source(source: &str) -> anyhow::Result<(Minifier, String)> {
        let mut module = TranslationUnit::from_str(source)?;
        let mut minifier = Minifier::default();
        minifier.minify(&mut module)?;
        Ok((minifier, minify_wgsl_source(&module.to_string())))
    }

    fn minify_test_data(name: &str) -> anyhow::Result<(Minifier, String)> {
        let source = read_test_data(&format!("{name}.input.wgsl"))?;
        minify_source(&source)
    }

    fn strip_wgsl_whitespace(input: &str) -> String {
        input.chars().filter(|c| !c.is_ascii_whitespace()).collect()
    }

    fn assert_wgsl_eq_ignoring_whitespace(actual: &str, expected: &str) {
        assert_eq!(
            strip_wgsl_whitespace(actual),
            strip_wgsl_whitespace(expected)
        );
    }

    fn assert_minified_matches_expected(name: &str) -> anyhow::Result<Minifier> {
        let (minifier, output) = minify_test_data(name)?;
        let expected = read_test_data(&format!("{name}.expected.wgsl"))?;
        assert_wgsl_eq_ignoring_whitespace(&output, &expected);
        Ok(minifier)
    }

    #[test]
    fn reuses_local_names_across_functions() -> anyhow::Result<()> {
        assert_minified_matches_expected("reuses_local_names_across_functions")?;
        Ok(())
    }

    #[test]
    fn write_map_excludes_locals_and_keeps_globals() -> anyhow::Result<()> {
        let name = "write_map_excludes_locals_and_keeps_globals";
        let minifier = assert_minified_matches_expected(name)?;

        let map_path = std::env::temp_dir().join(format!(
            "washi-map-test-{}-{}.map",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));

        minifier.write_map(&map_path)?;
        let map_contents = fs::read_to_string(&map_path)?;
        let _ = fs::remove_file(&map_path);

        let expected_map = read_test_data(&format!("{name}.expected.map"))?
            .replace("\r\n", "\n")
            .trim_end_matches('\n')
            .replace('\n', "\r\n");
        let expected_map = format!("{expected_map}\r\n");
        assert_eq!(map_contents, expected_map);

        Ok(())
    }

    #[test]
    fn resolves_local_before_global_in_value_context() -> anyhow::Result<()> {
        assert_minified_matches_expected("resolves_local_before_global_in_value_context")?;
        Ok(())
    }

    #[test]
    fn underscore_prefixed_idents_are_preserved() -> anyhow::Result<()> {
        let name = "underscore_prefixed_idents_are_preserved";
        let minifier = assert_minified_matches_expected(name)?;

        for (old, _) in &minifier.map_entries {
            assert!(!old.starts_with('_'));
        }

        Ok(())
    }

    #[test]
    fn function_params_do_not_shadow_module_scope_globals() -> anyhow::Result<()> {
        let (_minifier, output) = minify_test_data("function_params_do_not_shadow_module_scope_globals")?;

        // The body references the renamed globals; the parameters of `blur` must
        // therefore be allocated names that don't collide with those globals.
        let module = TranslationUnit::from_str(&output)?;
        let mut global_names = Vec::new();
        let mut blur_param_names = Vec::new();
        for decl in &module.global_declarations {
            match decl.node() {
                GlobalDeclaration::Declaration(d) => {
                    global_names.push(d.ident.to_string());
                }
                GlobalDeclaration::Function(f) => {
                    if f.attributes.is_empty() {
                        for p in &f.parameters {
                            blur_param_names.push(p.ident.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        assert_eq!(global_names.len(), 2);
        for global in &global_names {
            assert!(
                !blur_param_names.contains(global),
                "function parameter {blur_param_names:?} collided with module-scope global {global}"
            );
        }

        Ok(())
    }
}
