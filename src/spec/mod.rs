#![allow(unused_imports, unused_mut, dead_code)]

use std::collections::btree_map::{BTreeMap, Iter};
use std::io::{stderr, Write};

pub mod rustast;
mod fake_extctxt;

pub use self::fake_extctxt::with_fake_extctxt;
pub use self::grammar::{ParseError,specification};

peg_file! grammar("xdr.rustpeg");

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Value {
    Ident(String),
    Const(i64),
}

impl Value {
    fn as_ident(&self) -> rustast::Ident {
        match self {
            &Value::Ident(ref id) => rustast::str_to_ident(id),
            &Value::Const(val) => rustast::str_to_ident(&format!("Const{}{}",
                                                                 (if val < 0 { "_" } else { "" }),
                                                                 val.abs())),
        }
    }

    fn as_i64(&self, symtab: &Symtab) -> Option<i64> { symtab.value(self) }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Type {
    UInt,
    Int,
    UHyper,
    Hyper,
    Float,
    Double,
    Quadruple,
    Bool,

    // Special array elements
    Opaque,                     // binary
    String,                     // text

    // Compound types
    Enum(Vec<EnumDefn>),
    Struct(Vec<Decl>),
    Union(Box<Decl>, Vec<UnionCase>, Option<Box<Decl>>),

    Option(Box<Type>),
    Array(Box<Type>, Value),
    Flex(Box<Type>, Option<Value>),

    // Type reference (may be external)
    Ident(String),
}

impl Type {
    fn array(ty: Type, sz: Value) -> Type {
        Type::Array(Box::new(ty), sz)
    }

    fn flex(ty: Type, sz: Option<Value>) -> Type {
        Type::Flex(Box::new(ty), sz)
    }

    fn option(ty: Type) -> Type {
        Type::Option(Box::new(ty))
    }

    fn union((d, c, dfl): (Decl, Vec<UnionCase>, Option<Decl>)) -> Type {
        Type::Union(Box::new(d), c, dfl.map(Box::new))
    }

    fn is_boxed(&self, symtab: &Symtab) -> bool {
        use self::Type::*;

        match self {
            _ if self.is_prim(symtab) => false,
            &Array(_, _) | &Flex(_, _) | &Option(_) => false,
            &Ident(ref name) => if let Some(ty) = symtab.typedef(name) { ty.is_boxed(symtab) } else { true },
            _ => true,
        }
    }
    
    fn is_prim(&self, symtab: &Symtab) -> bool {
        use self::Type::*;

        match self {
            &Int | &UInt |
            &Hyper | &UHyper |
            &Float | &Double | &Quadruple |
            &Bool       => true,

            &Ident(ref id) => 
                match symtab.typedef(id) {
                    None => false,
                    Some(ref ty) => ty.is_prim(symtab),
                },
            
            _           => false,
        }
    }

    fn unpacker(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Vec<rustast::TokenTree> {
        use self::Type::*;

        match self {
            &Array(_, ref value) => {
                let elems = value.as_i64(symtab).unwrap();
                let mut unpacks = Vec::new();

                // I think this is the only way to safely initialize a fixed-sized array in
                // Rust. The alternative would be to use a Vec<>, but this would need to deal with a
                // wrong-sized value, and an extra indirection.
                for _ in 0..elems {
                    unpacks.push(quote_tokens!(ctxt, { let (v, esz) = try!(xdr::Unpack::unpack(input)); asz += esz; v },));
                }

                quote_tokens!(ctxt, { let mut asz = 0; let v = [ $unpacks ]; (v, asz) })
            },

            _ => quote_tokens!(ctxt, try!(xdr::Unpack::unpack(input))),
        }
    }
    
    fn as_token(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Vec<rustast::TokenTree> {
        use self::Type::*;
        
        match self {
            &Int        => quote_tokens!(ctxt, i32),            
            &UInt       => quote_tokens!(ctxt, u32),            
            &Hyper      => quote_tokens!(ctxt, i64),            
            &UHyper     => quote_tokens!(ctxt, u64),            
            &Float      => quote_tokens!(ctxt, f32),            
            &Double     => quote_tokens!(ctxt, f64),            
            &Quadruple  => quote_tokens!(ctxt, f128),            
            &Bool       => quote_tokens!(ctxt, bool),

            &Option(box ref ty) => {
                let tok = ty.as_token(symtab, ctxt);
                if ty.is_boxed(symtab) {
                    quote_tokens!(ctxt, Option<Box<$tok>>)
                } else {
                    quote_tokens!(ctxt, Option<$tok>)
                }
            },

            &Array(box String, _) => quote_tokens!(ctxt, String),
            &Array(box Opaque, ref val) => {
                match val {
                    &Value::Const(c) => {
                        let c = c as usize;
                        quote_tokens!(ctxt, [u8; $c])
                    },
                    &Value::Ident(ref id) => {
                        let tok = rustast::str_to_ident(id);
                        if let Some((_, Some(ref scope))) = symtab.getconst(id) {
                            let scope = rustast::str_to_ident(scope);
                            quote_tokens!(ctxt, [u8; $scope::$tok as usize])
                        } else {
                            quote_tokens!(ctxt, [u8; $tok as usize])
                        }
                    }
                }
            },
            
            &Array(box ref ty, ref val) => {
                let tytok = ty.as_token(symtab, ctxt);
                match val {
                    &Value::Const(c) => {
                        let c = c as usize;
                        quote_tokens!(ctxt, [$tytok; $c])
                    },
                    &Value::Ident(ref id) => {
                        let tok = rustast::str_to_ident(id);
                        if let Some((_, Some(ref scope))) = symtab.getconst(id) {
                            let scope = rustast::str_to_ident(scope);
                            quote_tokens!(ctxt, [$tytok; $scope::$tok as usize])
                        } else {
                            quote_tokens!(ctxt, [$tytok; $tok as usize])
                        }
                    }
                }
            },

            &Flex(box String, _) => quote_tokens!(ctxt, String),
            &Flex(box Opaque, _) => quote_tokens!(ctxt, Vec<u8>),
            &Flex(box ref ty, _) => {
                let tok = ty.as_token(symtab, ctxt);
                quote_tokens!(ctxt, Vec<$tok>)
            },
            
            &Ident(ref name) => {
                let id = rustast::str_to_ident(name);
                quote_tokens!(ctxt, $id)
            },
            
            _ => panic!("can't have unnamed type {:?}", self),
        }
    }        
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct EnumDefn(pub String, pub Option<Value>);

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct UnionCase(Value, Decl);

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Decl {
    Void,
    Named(String, Type),
}

impl Decl {
    fn as_token(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Option<(rustast::Ident, Vec<rustast::TokenTree>)> {
        use self::Decl::*;
        use self::Type::*;
        match self {
            &Void => None,
            &Named(ref name, ref ty) => {
                let mut tok = ty.as_token(symtab, ctxt);
                if false && ty.is_boxed(symtab) {
                    tok = quote_tokens!(ctxt, Box<$tok>)
                };
                Some((rustast::str_to_ident(name), tok))
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Typedef(pub String, pub Type);

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Const(pub String, pub i64);

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Defn {
    Typedef(String, Type),
    Const(String, i64),
}

pub trait Emit {
    fn define(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> rustast::P<rustast::Item>;
}

pub trait Emitpack : Emit {
    fn pack(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Option<rustast::P<rustast::Item>>;
    fn unpack(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Option<rustast::P<rustast::Item>>;
}

impl Emit for Const {
    fn define(&self, _: &Symtab, ctxt: &rustast::ExtCtxt) -> rustast::P<rustast::Item> {
        let name = rustast::str_to_ident(&self.0);
        let val = &self.1;
        
        quote_item!(ctxt, pub const $name: i64 = $val;).unwrap()
    }
}

impl Emit for Typedef {
    fn define(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> rustast::P<rustast::Item> {
        use self::Type::*;
        
        let name = rustast::str_to_ident(&self.0);
        let ty = &self.1;

        match ty {
            &Enum(ref edefs) => {
                let defs: Vec<_> = edefs.iter()
                    .filter_map(|&EnumDefn(ref field, _)| {
                        if let Some((val, Some(_))) = symtab.getconst(field) {
                            Some((rustast::str_to_ident(&field), val as isize))
                        } else {
                            None
                        }
                    })
                    .map(|(field, val)| quote_tokens!(ctxt, $field = $val,))
                    .collect();
                
                quote_item!(ctxt, #[derive(Debug, Eq, PartialEq)] pub enum $name { $defs }).unwrap()
            },

            &Struct(ref decl) => {
                use self::Decl::*;
                
                let decls: Vec<_> = decl.iter()
                    .filter_map(|d| d.as_token(symtab, ctxt))
                    .map(|(field, ty)| quote_tokens!(ctxt, pub $field: $ty,))
                    .collect();

                quote_item!(ctxt,
                            #[derive(Debug,Eq,PartialEq)]
                            pub struct $name { $decls }).unwrap()
            },

            &Union(box ref selector, ref cases, ref defl) => {
                use self::Decl::*;
                use self::Value::*;

                let labelfields = false; // true - include label in enum branch

                // return true if case is compatible with the selector
                let compatcase = |case: &Value| {
                    let seltype = match selector {
                        &Void => return false,
                        &Named(_, ref ty) => ty
                    };

                    match case {
                        &Const(val) if val < 0 =>
                            match seltype {
                                &Int | &Hyper => true,
                                _ => false,
                            },

                        &Const(_) =>
                            match seltype {
                                &Int | &Hyper | &UInt | &UHyper => true,
                                _ => false,
                            },

                        &Ident(ref id) =>
                            if *seltype == Bool {
                                id == "TRUE" || id == "FALSE"
                            } else {
                                if let &Type::Ident(ref selname) = seltype {
                                    match symtab.getconst(id) {
                                        Some((_, Some(ref scope))) => scope == selname,
                                        _ => false,
                                    }
                                } else {
                                    false
                                }
                            },
                    }
                };

                let mut cases: Vec<_> = cases.iter()
                    .map(|&UnionCase(ref val, ref decl)| {
                        if !compatcase(val) {
                            println!("// incompat selector {:?} case {:?}", selector, val);
                            return quote_tokens!(ctxt, );
                        }

                        let label = val.as_ident();

                        match decl {
                            &Void => quote_tokens!(ctxt, $label,),
                            &Named(ref name, ref ty) => {
                                let mut tok = ty.as_token(symtab, ctxt);
                                if ty.is_boxed(symtab) {
                                    tok = quote_tokens!(ctxt, Box<$tok>)
                                };
                                if labelfields {
                                    let name = rustast::str_to_ident(name);
                                    quote_tokens!(ctxt, $label { $name : $tok },)
                                } else {
                                    quote_tokens!(ctxt, $label($tok),)
                                }
                            }
                        }
                    })
                    .collect();

                if let &Some(box Named(ref name, ref ty)) = defl {
                    let mut tok = ty.as_token(symtab, ctxt);
                    if ty.is_boxed(symtab) {
                        tok = quote_tokens!(ctxt, Box<$tok>)
                    };
                    if labelfields {
                        let name = rustast::str_to_ident(name);
                        cases.push(quote_tokens!(ctxt, default { $name: $tok },))
                    } else {
                        cases.push(quote_tokens!(ctxt, default($tok),))
                    }
                }

                quote_item!(ctxt,
                            #[derive(Debug, Eq, PartialEq)]
                            pub enum $name { $cases }).unwrap()
            },

            _ => {
                let tok = ty.as_token(symtab, ctxt);
                quote_item!(ctxt, pub type $name = $tok;).unwrap()
            },
        }
    }
}

impl Emitpack for Typedef {
    fn pack(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Option<rustast::P<rustast::Item>> {
        use self::Type::*;
        use self::Decl::*;

        let name = rustast::str_to_ident(&self.0);
        let ty = &self.1;

        let body: Vec<rustast::TokenTree> = match ty {
            &Enum(_) | &Ident(_) => return None,
            _ if ty.is_prim(symtab) => return None,

            &Struct(ref decl) => {
                let decls: Vec<_> = decl.iter()
                    .filter_map(|d| d.as_token(symtab, ctxt))
                    .map(|(field, _)| quote_tokens!(ctxt, try!(self.$field.pack(out)) + ))
                    .collect();
                quote_tokens!(ctxt, $decls 0)
            },

            &Union(_, ref cases, ref defl) => {
                let mut matches: Vec<_> = cases.iter()
                    .filter_map(|&UnionCase(ref val, ref decl)| {
                        let label = val.as_ident();
                        let disc = match val.as_i64(symtab) {
                            Some(v) => v as i32,
                            None => return None
                        };
                        
                        let ret = match decl {
                            &Void => quote_tokens!(ctxt,
                                                   $name::$label => try!($disc.pack(out)),),
                            &Named(_, _) => quote_tokens!(ctxt,
                                                          $name::$label(val) => try!($disc.pack(out)) + try!(val.pack(out)),),
                        };
                        Some(ret)
                    })
                    .collect();

                if let &Some(box ref decl) = defl {
                    let default =
                        match decl {
                            &Void => quote_tokens!(ctxt, $name::default => return Err(xdr::Error::invalidcase()),),
                            &Named(_, _) => quote_tokens!(ctxt, $name::default(_) => return Err(xdr::Error::invalidcase()),),
                        };

                    matches.push(default)
                }

                quote_tokens!(ctxt, match self { $matches })
            },

            &Flex(_, _) | &Option(_) | &Array(_, _) =>
                quote_tokens!(ctxt, try!(self.pack(input))),
            
            _ => panic!("unimplemented ty={:?}", ty)
        };

        quote_item!(ctxt,
                    impl<Out: xdr::Write> xdr::Pack<Out> for $name {
                        fn pack(&self, out: &mut Out) -> xdr::Result<usize> {
                            Ok($body)
                        }
                    })
    }

    fn unpack(&self, symtab: &Symtab, ctxt: &rustast::ExtCtxt) -> Option<rustast::P<rustast::Item>> {
        use self::Type::*;
        use self::Decl::*;
        
        let name = rustast::str_to_ident(&self.0);
        let ty = &self.1;

        let body = match ty {
            &Enum(_) | &Ident(_) => return None,
            _ if ty.is_prim(symtab) => return None,
            
            &Struct(ref decl) => {
                let decls: Vec<_> = decl.iter()
                    .filter_map(|d| match d {
                        &Void => None,
                        &Named(_, ref ty) => {
                            if let Some((tok, _)) = d.as_token(symtab, ctxt) {
                                Some((tok, ty))
                            } else {
                                None
                            }
                        },
                    })
                    .map(|(field, ty)| {
                        let unpack = ty.unpacker(symtab, ctxt);
                        quote_tokens!(ctxt, $field: { let (v, fsz) = $unpack; sz += fsz; v },)
                    })
                    .collect();
                
                quote_tokens!(ctxt, $name { $decls })
            },

            &Union(_, ref cases, ref defl) => {
                let mut matches: Vec<_> = cases.iter()
                    .filter_map(|&UnionCase(ref val, ref decl)| {
                        let label = val.as_ident();
                        let disc = match val.as_i64(symtab) {
                            Some(v) => v as i32,
                            None => return None
                        };
                        
                        let ret = match decl {
                            &Void => quote_tokens!(ctxt, $disc => $name::$label,),
                            &Named(_, _) => quote_tokens!(ctxt,
                                                          $disc => $name::$label({
                                                              let (v, csz) = try!(xdr::Unpack::unpack(input));
                                                              sz += csz;
                                                              v
                                                          }),),
                        };
                        Some(ret)
                    })
                    .collect();

                if let &Some(box ref decl) = defl {
                    let defl = match decl {
                        &Void => quote_tokens!(ctxt, _ => $name::default),
                        &Named(_, _) => quote_tokens!(ctxt, _ => $name::default({
                            let (v, csz) = try!(xdr::Unpack::unpack(input));
                            sz += csz;
                            v
                        })),
                    };

                    matches.push(defl);
                } else {
                    let defl = quote_tokens!(ctxt, _ => return Err(xdr::Error::invalidcase()));
                    matches.push(defl);
                }

                quote_tokens!(ctxt, match { let (v, dsz) = try!(xdr::Unpack::unpack(input)); sz += dsz; v } { $matches })
            },

            &Flex(_, _) | &Option(_) => quote_tokens!(ctxt, { let (v, asz) = try!(xdr::Unpack::unpack(input)); sz += asz; v }),

            &Array(_, _) => ty.unpacker(symtab, ctxt),

            _ => panic!("unimplemented ty={:?}", ty)
        };

        quote_item!(ctxt,
                    impl<In: xdr::Read> xdr::Unpack<In> for $name {
                        fn unpack(input: &mut In) -> xdr::Result<($name, usize)> {
                            let mut sz = 0;
                            Ok(($body, sz))
                        }
                    })
    }
}

#[derive(Debug, Clone)]
pub struct Symtab {
    consts: BTreeMap<String, (i64, Option<String>)>,
    typedefs: BTreeMap<String, Type>,
}

impl Symtab {
    pub fn new(defns: &Vec<Defn>) -> Symtab {
        let mut ret = Symtab {
            consts: BTreeMap::new(),
            typedefs: BTreeMap::new(),
        };

        ret.update_consts(&defns);
        
        ret
    }

    fn update_consts(&mut self, defns: &Vec<Defn>) {
        for defn in defns {
            match defn {
                &Defn::Typedef(ref name, ref ty) => {
                    self.deftype(name, ty);
                    self.update_enum_consts(name, ty);
                },

                &Defn::Const(ref name, val) => {
                    self.defconst(name, val, None)
                },
            }
        }
    }

    fn update_enum_consts(&mut self, scope: &String, ty: &Type) {
        let mut err = stderr();
        let mut prev = -1;
        
        if let &Type::Enum(ref edefn) = ty {
            for &EnumDefn(ref name, ref maybeval) in edefn {
                let v = match maybeval {
                    &None => prev + 1,
                    &Some(ref val) => match self.value(val) {
                        Some(c) => c,
                        None => { let _ = writeln!(&mut err, "Unknown value {:?}", val); continue }
                    }
                };
                
                prev = v;

                //println!("enum {} -> {}", name, v);
                self.defconst(name, v, Some(scope.clone()));
            }
        }
    }
    
    fn defconst(&mut self, name: &String, val: i64, scope: Option<String>) {
        self.consts.insert(name.clone(), (val, scope));
    }

    fn deftype(&mut self, name: &String, ty: &Type) {
        self.typedefs.insert(name.clone(), ty.clone());
    }

    pub fn getconst(&self, name: &String) -> Option<(i64, Option<String>)> {
        match self.consts.get(name) {
            None => None,
            Some(c) => Some(c.clone()),
        }
    }

    pub fn value(&self, val: &Value) -> Option<i64> {
        match val {
            &Value::Const(c) => Some(c),
            &Value::Ident(ref id) => self.getconst(id).map(|(v, _)| v),
        }        
    }

    pub fn typedef(&self, name: &String) -> Option<&Type> {
        match self.typedefs.get(name) {
            None => None,
            Some(ty) => Some(ty),
        }
    }

    pub fn constants(&self) -> Iter<String, (i64, Option<String>)> {
        self.consts.iter()
    }

    pub fn typedefs(&self) -> Iter<String, Type> {
        self.typedefs.iter()
    }
}


#[cfg(test)]
mod test;
