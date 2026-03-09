use clap::{Parser, Subcommand};
use itertools::Itertools;
use shadow_rs::shadow;
use std::collections::HashMap;

pub mod format;

use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use wgsl_parse::syntax::{
    CaseSelector, CompoundStatement, Expression, ExpressionNode,
    GlobalDeclaration, Ident, Statement, StatementNode, TranslationUnit, TypeExpression,
};
use wgsl_types::idents::{RESERVED_WORDS, iter_builtin_idents};
use crate::format::minify_wgsl_source;

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
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Minify {
            input_filename,
            output_filename,
        } => {
            fs::write(
                output_filename,
                minify(&fs::read_to_string(&input_filename)?)?,
            )?;
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
        id.chars().all(|c| SWIZZLES.contains(&c)) || RESERVED_WORDS.contains(&id) || iter_builtin_idents().contains(id)
    }

    pub fn next_ident(&mut self) -> Ident {
        Ident::new(self.next())
    }
}

pub fn minify(wgsl: &str) -> anyhow::Result<String> {
    let mut module = TranslationUnit::from_str(&wgsl)?;
    let mut minifier = Minifier::default();
    minifier.minify(&mut module)?;

    Ok(minify_wgsl_source(&module.to_string()))
    //Ok(module.to_string())
}

#[derive(Default)]
pub struct Minifier {
    id: Identifier,
    ident_map: HashMap<String, Ident>,
}

impl Minifier {
    pub fn minify(&mut self, module: &mut TranslationUnit) -> anyhow::Result<()> {
        for decl in &mut module.global_declarations {
            match decl.node_mut() {
                GlobalDeclaration::Struct(s) => {
                    self.map_ident(&mut s.ident);

                    for m in &mut s.members {
                        self.map_ident(&mut m.ident);
                    }
                }
                GlobalDeclaration::Declaration(d) => {
                    if let Some(t) = &mut d.ty {
                        self.minify_type_expression(t)?;
                    }
                    if let Some(expr) = &mut d.initializer {
                        self.minify_expr(expr)?;
                    }

                    self.map_ident(&mut d.ident);
                }
                GlobalDeclaration::Function(f) => {
                    if f.attributes.is_empty() {
                        self.map_ident(&mut f.ident);
                    } else {
                        // Has attributes so is probably an entry point
                        self.no_map(&mut f.ident);
                    }

                    for p in &mut f.parameters {
                        self.map_ident(&mut p.ident);
                        self.minify_type_expression(&mut p.ty)?;
                    }

                    self.minify_compound_statement(&mut f.body)?;

                    if let Some(t) = &mut f.return_type {
                        self.minify_type_expression(t)?;
                    }
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
                self.minify_type_expression(&mut stmt.call.ty)?;
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
                self.map_ident(&mut stmt.ident);
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
                self.map_ident(&mut expr.component);
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
                self.minify_type_expression(&mut expr.ty)?;
            }
            Expression::TypeOrIdentifier(expr) => {
                self.minify_type_expression(expr)?;
            }
        }

        Ok(())
    }

    fn map_ident(&mut self, ident: &mut Ident) {
        let old = ident.to_string();

        if Identifier::is_reserved(&old) {
            self.no_map(ident);
            return;
        }

        if self.ident_map.contains_key(&old) {
            *ident = self.ident_map.get(&old).unwrap().clone();
            return;
        }

        let new_ident = self.id.next_ident();
        self.ident_map.insert(old, new_ident.clone());
        *ident = new_ident;
    }

    fn no_map(&mut self, ident: &mut Ident) {
        self.ident_map.insert(ident.to_string(), ident.clone());
    }

    fn minify_type_expression(&mut self, t: &mut TypeExpression) -> anyhow::Result<()> {
        match &mut t.template_args {
            Some(templ) => {
                for t in templ {
                    self.minify_expr(&mut t.expression)?;
                }
            }
            None => {
                self.map_ident(&mut t.ident);
            }
        }

        Ok(())
    }

    fn minify_compound_statement(&mut self, stmt: &mut CompoundStatement) -> anyhow::Result<()> {
        for stmt in &mut stmt.statements {
            self.minify_stmt(stmt)?;
        }
        Ok(())
    }
}
