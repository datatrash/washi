use clap::{Parser, Subcommand};
use shadow_rs::shadow;

pub mod format;

use naga::back::wgsl::WriterFlags;
use naga::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use naga::compact::{compact, KeepUnused};
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
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Minify {
            input_filename, output_filename
        } => {
            let m = minify(&fs::read_to_string(&input_filename)?)?;

            // Minify again to also minify the local variables Naga inserts
            let m2 = minify(&m)?;

            fs::write(output_filename, m2)?;
        }
    }

    Ok(())
}

pub struct Identifier(usize);

impl Identifier {
    pub fn new() -> Self {
        Self(0)
    }

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
            if !keywords::wgsl::RESERVED_SET.contains(&id) {
                return id;
            }
        }
    }
}

pub fn minify(wgsl: &str) -> anyhow::Result<String> {
    let mut module: Module = front::wgsl::parse_str(wgsl)?;

    let module_info: valid::ModuleInfo =
        valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::all())
            .subgroup_stages(valid::ShaderStages::all())
            .subgroup_operations(valid::SubgroupOperationSet::all())
            .validate(&module)?;

    compact(&mut module, KeepUnused::No);

    let mut m = Minifier::new();

    for (_, g) in module.global_variables.iter_mut() {
        m.minify_global(g);
    }

    for (_, f) in module.functions.iter_mut() {
        m.minify_function(f, true);
    }

    for ep in module.entry_points.iter_mut() {
        m.minify_function(&mut ep.function, false);
    }

    m.minify_types(&mut module);

    let mut out = String::new();
    back::wgsl::Writer::new(&mut out, WriterFlags::empty()).write(&module, &module_info)?;

    Ok(minify_wgsl_source(&out))
}

pub struct Minifier {
    id: Identifier,
    type_map: HashMap<Handle<Type>, Handle<Type>>,
}

impl Minifier {
    pub fn new() -> Self {
        Self {
            id: Identifier::new(),
            type_map: HashMap::new(),
        }
    }

    pub fn minify_global(&mut self, g: &mut GlobalVariable) {
        if let Some(name) = &mut g.name {
            *name = self.id.next();
        }
    }

    pub fn minify_function(&mut self, f: &mut Function, minify_name: bool) {
        if minify_name && let Some(name) = &mut f.name {
            *name = self.id.next();
        }

        for arg in &mut f.arguments {
            if let Some(name) = &mut arg.name {
                *name = self.id.next();
            }
        }

        for (_, lv) in &mut f.local_variables.iter_mut() {
            if let Some(name) = &mut lv.name {
                *name = self.id.next();
            }
        }

        for (_, ne) in f.named_expressions.iter_mut() {
            *ne = self.id.next();
        }
    }

    pub fn minify_types(&mut self, module: &mut Module) {
        let mut new_types = UniqueArena::new();

        for (old_handle, ty) in module.types.iter() {
            let mut new_ty = ty.clone();

            if new_ty.name.is_some() {
                new_ty.name = Some(self.id.next());
            }

            // Update internal handles (for Structs, Arrays, Pointers)
            self.update_type_inner(&mut new_ty.inner);

            let new_handle = new_types.insert(new_ty, module.types.get_span(old_handle));
            self.type_map.insert(old_handle, new_handle);
        }
        module.types = new_types;

        for (_, constant) in module.constants.iter_mut() {
            constant.ty = self.remap(constant.ty);
        }

        for (_, global) in module.global_variables.iter_mut() {
            global.ty = self.remap(global.ty);
        }

        for (_, func) in module.functions.iter_mut() {
            self.update_function(func);
        }

        for ep in &mut module.entry_points {
            self.update_function(&mut ep.function);
        }
    }

    fn remap(&self, old: Handle<Type>) -> Handle<Type> {
        *self.type_map.get(&old).expect("Orphaned type handle found")
    }

    fn update_type_inner(&mut self, inner: &mut TypeInner) {
        match inner {
            TypeInner::Array { base, .. } => *base = self.remap(*base),
            TypeInner::Pointer { base, .. } => *base = self.remap(*base),
            TypeInner::Struct { members, .. } => {
                for member in members {
                    if let Some(name) = &mut member.name {
                        *name = self.id.next();
                    }
                    member.ty = self.remap(member.ty);
                }
            }
            TypeInner::BindingArray { base, .. } => *base = self.remap(*base),
            _ => {}
        }
    }

    fn update_function(&self, func: &mut Function) {
        // Return type
        if let Some(ref mut result) = func.result {
            result.ty = self.remap(result.ty);
        }

        // Arguments
        for arg in &mut func.arguments {
            arg.ty = self.remap(arg.ty);
        }

        // Local Variables
        for (_, local) in func.local_variables.iter_mut() {
            local.ty = self.remap(local.ty);
        }

        // Expressions
        for (_, expr) in func.expressions.iter_mut() {
            match expr {
                Expression::Compose { ty, .. } => *ty = self.remap(*ty),
                _ => {}
            }
        }
    }
}
