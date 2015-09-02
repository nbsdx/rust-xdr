//! XDR codec generation
//!
//! This crate provides library interfaces for programatically generating Rust code to implement
//! RFC4506 XDR encoding/decoding, as well as a command line tool "xdrgen".
//!
//! It is intended to be used with the "xdr-codec" crate, which provides the runtime library for
//! encoding/decoding primitive types, strings, opaque data and arrays.
#![feature(slice_patterns, plugin, rustc_private, quote, box_patterns)]
#![plugin(peg_syntax_ext)]
#![crate_type = "lib"]
extern crate syntax;
extern crate xdr_codec as xdr;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::io::{Read, Write};
use std::error::Error;
use std::fmt::Display;
use std::env;

use xdr::Result;

mod spec;
use spec::{Symtab, Emit, Emitpack};
use spec::{with_fake_extctxt, rustast, specification};

/// Generate Rust code from an RFC4506 XDR specification
///
/// `infile` is simply a string used in error messages; it may be empty. `input` is a read stream of
/// the specification, and `output` is where the generated code is sent.
pub fn generate<In, Out>(infile: &str, mut input: In, mut output: Out) -> Result<()>
    where In: Read, Out: Write
{
    let mut source = String::new();

    try!(input.read_to_string(&mut source));

    let xdr = match spec::specification(&source) {
        Ok(defns) => Symtab::new(&defns),
        Err(err) => return Err(xdr::Error::from(err.description())),
    };
    
    with_fake_extctxt(|e| {
        let consts = xdr.constants()
            .filter_map(|(c, &(v, ref scope))| {
                if scope.is_none() {
                    Some(spec::Const(c.clone(), v))
                } else {
                    None
                }
            })
            .map(|c| c.define(&xdr, e));

        let typedefs = xdr.typedefs()
            .map(|(n, ty)| spec::Typedef(n.clone(), ty.clone()))
            .map(|c| c.define(&xdr, e));

        let packers = xdr.typedefs()
            .map(|(n, ty)| spec::Typedef(n.clone(), ty.clone()))
            .filter_map(|c| c.pack(&xdr, e));
        
        let unpackers = xdr.typedefs()
            .map(|(n, ty)| spec::Typedef(n.clone(), ty.clone()))
            .filter_map(|c| c.unpack(&xdr, e));

        let module = consts.chain(typedefs).chain(packers).chain(unpackers);

        let _ = writeln!(output, r#"
// GENERATED CODE
//
// Generated from {}.
//
// DO NOT EDIT

"#, infile);
        for it in module {
            let _ = writeln!(output, "{}\n", rustast::item_to_string(&*it));
        }
    });

    Ok(())
}

/// Simplest possible way to generate Rust code from an XDR specification.
///
/// It is intended for use in a build.rs script:
///
/// ```ignore
/// extern crate xdrgen;
/// 
/// fn main() {
///    xdrgen::compile("src/simple.x").unwrap();
/// }
/// ```
///
/// Output is put into OUT_DIR, and can be included:
///
/// ```ignore
/// mod simple {
///    use xdr_codec;
///    
///    include!(concat!(env!("OUT_DIR"), "/simple_xdr.rs"));
/// }
/// ```
///
/// If your specification uses types which are not within the specification, you can provide your
/// own implementations of `Pack` and `Unpack` for them.
pub fn compile<P>(infile: P) -> Result<()>
    where P: AsRef<Path> + Display
{
    let input = try!(File::open(&infile));

    let mut outdir = PathBuf::from(env::var("OUT_DIR").unwrap_or(String::from(".")));
    let outfile = PathBuf::from(infile.as_ref()).file_stem().unwrap().to_owned().into_string().unwrap().replace("-", "_");

    outdir.push(&format!("{}_xdr.rs", outfile));
    
    let output = try!(File::create(outdir));

    generate(infile.as_ref().as_os_str().to_str().unwrap_or("<unknown>"), input, output)
}