use crate::{DefaultGeneric, ImplArgs, Mode};
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parse_quote, punctuated::Punctuated, ConstParam, Error, Expr, GenericParam, Ident, ItemImpl,
    PathArguments, Token, Type, TypeParam, TypePath,
};

#[derive(Clone)]
enum Generic {
    Type {
        ident: Ident,
        default: Option<Ident>,
    },
    Const(Ident),
}

impl ToTokens for Generic {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Generic::Type { ident, default } => {
                if let Some(default) = default {
                    tokens.extend(quote!(#default));
                } else {
                    tokens.extend(quote!(#ident));
                }
            }
            Generic::Const(ident) => {
                tokens.extend(quote!(#ident));
            }
        }
    }
}

pub(crate) fn expand(args: ImplArgs, mut input: ItemImpl, mode: Mode) -> TokenStream {
    // Parse name
    let name = match args.name {
        Some(name) => name.parse().unwrap(),
        None => match type_name(&input.self_ty) {
            Some(name) => name,
            None => {
                let msg = "use #[typetag::serde(name = \"...\")] to specify a unique name";
                return Error::new_spanned(&input.self_ty, msg).to_compile_error();
            }
        },
    }
    .to_token_stream();

    let name_lower = name.to_string().to_lowercase();
    let name_lower = syn::Ident::new(&name_lower, Span::call_site()).to_token_stream();
    let name_quotes = name.to_string();
    let name_quotes = quote!(#name_quotes);

    // Parse generics
    let generics = process_generics(&input.generics.params, &args.generics);
    let needed_generics: Vec<_> = generics
        .iter()
        .filter(|g| {
            !matches!(
                g,
                Generic::Type {
                    default: Some(_),
                    ..
                }
            )
        })
        .collect();

    // Add stuff to the impl
    augment_impl(&mut input, &generics, &name, mode);

    let mut expanded = quote! {
        #input
    };

    let object = &input.trait_.as_ref().unwrap().1;
    // Get name of type without generics
    let path = match &input.self_ty.as_ref() {
        Type::Path(path) => {
            let mut p = path.path.clone();
            p.segments.last_mut().unwrap().arguments = PathArguments::None;
            p.to_token_stream()
        }
        _ => input.self_ty.to_token_stream(),
    };

    if mode.de {
        if needed_generics.is_empty() {
            expanded.extend(quote! {
                typetag::__private::inventory::submit! {
                    <dyn #object>::typetag_register(
                        #name_quotes,
                        (|deserializer| typetag::__private::Result::Ok(
                            typetag::__private::Box::new(
                                typetag::__private::erased_serde::deserialize::<#path<#(#generics),*>>(deserializer)?
                            ),
                        )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                    )
                }
            });
        } else {
            // Add in macro
            println!("object {}", object.to_token_stream());
            expanded.extend(quote! {
                mod register {
                    #[allow(unused_macros)]
                    macro_rules! #name_lower {
                        ($(<#($#needed_generics:tt),*>),* $(,)?) => {$(
                            typetag::__private::inventory::submit! {
                                <dyn #object>::typetag_register(
                                    concat!(#name_quotes, "<", #(stringify!($#generics)),*, ">"),
                                    (|deserializer| typetag::__private::Result::Ok(
                                        typetag::__private::Box::new(
                                            typetag::__private::erased_serde::deserialize::<#path<#($#generics),*>>(deserializer)?
                                        ),
                                    )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                                )
                            }
                        )*}
                    }
                    pub(crate) use #name_lower;
                }
            });
        }
    }

    expanded
}

fn augment_impl(input: &mut ItemImpl, generics: &[Generic], name: &TokenStream, mode: Mode) {
    if mode.ser {
        // Fill out where clauses
        for g in generics {
            match g {
                Generic::Type { ident, default } => match default {
                    Some(_) => {
                        match &mut input.generics.where_clause {
                            Some(ref mut wc) => {
                                wc.predicates.push(parse_quote!(#ident: serde::Serialize))
                            }
                            None => {
                                input.generics.where_clause =
                                    Some(parse_quote!(where #ident: serde::Serialize))
                            }
                        };
                    }
                    None => {
                        match &mut input.generics.where_clause {
                            Some(ref mut wc) => {
                                wc.predicates.push(parse_quote!(#ident: typetag::Tagged))
                            }
                            None => {
                                input.generics.where_clause =
                                    Some(parse_quote!(where #ident: typetag::Tagged))
                            }
                        };
                    }
                },
                Generic::Const(_) => {}
            }
        }

        // Fill out extra function definitions
        let mut args = Punctuated::<Expr, Token![,]>::new();
        for g in generics {
            match g {
                Generic::Type { ident, default } => {
                    match default {
                        Some(_) => {}
                        None => args.push(parse_quote!(<#ident as typetag::Tagged>::tag())),
                    };
                }
                Generic::Const(ident) => args.push(parse_quote!(#ident)),
            }
        }

        if args.is_empty() {
            let name_quotes = name.to_string();
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    String::from(#name_quotes)
                }
            });
        } else {
            let form = format!("{}<{}>", name, "{}".repeat(args.len()));
            input.items.push(parse_quote!(
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    format!(#form, #args)
                }
            ));
        }
    }

    if mode.de {
        input.items.push(parse_quote! {
            #[doc(hidden)]
            fn typetag_deserialize(&self) {}
        });
    }
}

// Merge generics from the impl and from args
fn process_generics(
    input: &Punctuated<GenericParam, Token![,]>,
    defaults: &[DefaultGeneric],
) -> Vec<Generic> {
    let mut generics = Vec::new();

    for param in input {
        match param {
            GenericParam::Type(TypeParam { ident, .. }) => {
                let default = defaults
                    .iter()
                    .find(|d| &d.typename == ident)
                    .map(|d| d.default.clone());
                generics.push(Generic::Type {
                    ident: ident.clone(),
                    default,
                });
            }
            GenericParam::Const(ConstParam { ident, .. }) => {
                generics.push(Generic::Const(ident.clone()));
            }
            GenericParam::Lifetime(_) => {}
        }
    }
    generics
}

fn type_name(mut ty: &Type) -> Option<Ident> {
    loop {
        match ty {
            Type::Path(TypePath { qself: None, path }) => {
                return Some(path.segments.last().unwrap().ident.clone());
            }
            Type::Group(group) => {
                ty = &group.elem;
            }
            _ => return None,
        }
    }
}
