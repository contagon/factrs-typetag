use crate::{ImplArgs, Mode};
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parse_quote, punctuated::Punctuated, ConstParam, Error, Expr, GenericParam, Ident, ItemImpl,
    Token, Type, TypeParam, TypePath,
};

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

    let name_quotes = name.to_string();
    let name_quotes = quote!(#name_quotes);

    // Parse generics
    let generics = input
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(_) => Some(p),
            GenericParam::Const(_) => Some(p),
            GenericParam::Lifetime(_) => None,
        })
        .cloned()
        .collect::<Vec<_>>();

    // Add stuff to the impl
    // TODO: Need to make it work if entire name is a generic as well
    augment_impl(&mut input, &generics, &name, mode);

    let mut expanded = quote! {
        #input
    };

    let object = &input.trait_.as_ref().unwrap().1;

    if mode.de {
        // If no generics, register the type directly
        if generics.is_empty() {
            let self_ty = &input.self_ty.as_ref();
            expanded.extend(quote! {
                typetag::__private::inventory::submit! {
                    <dyn #object>::typetag_register(
                        #name_quotes,
                        (|deserializer| typetag::__private::Result::Ok(
                            typetag::__private::Box::new(
                                typetag::__private::erased_serde::deserialize::<#self_ty>(deserializer)?
                            ),
                        )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                    )
                }
            });
        } else {
            // Get all generics as idents
            let generic_ident = generics
                .into_iter()
                .map(|p| match p {
                    GenericParam::Type(tp) => tp.ident,
                    GenericParam::Const(cp) => cp.ident,
                    _ => panic!("unexpected type"),
                })
                .collect::<Vec<_>>();
            // Extract name of type
            let path = match &input.self_ty.as_ref() {
                Type::Path(path) => path.path.segments.last().unwrap().ident.clone(),
                _ => {
                    let msg = "generics only supported on paths";
                    return Error::new_spanned(&input.self_ty, msg).to_compile_error();
                }
            };

            // If blanket impl, name macro after trait
            if generic_ident.len() == 1 && generic_ident.contains(&path) {
                let trait_lower = &object
                    .segments
                    .last()
                    .unwrap()
                    .ident
                    .to_string()
                    .to_lowercase();
                let trait_lower = syn::Ident::new(&trait_lower, Span::call_site());
                expanded.extend(quote! {
                    #[allow(unused_macros)]
                    macro_rules! #trait_lower {
                        ($($kind:tt),* $(,)?) => {$(
                            typetag::__private::inventory::submit! {
                                <dyn #object>::typetag_register(
                                    stringify!($kind),
                                    (|deserializer| typetag::__private::Result::Ok(
                                        typetag::__private::Box::new(
                                            typetag::__private::erased_serde::deserialize::<$kind>(deserializer)?
                                        ),
                                    )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                                )
                            }
                        )*}
                    }
                    pub(crate) use #trait_lower;
                });
            // Otherwise, name it after type
            } else {
                let name_lower = name.to_string().to_lowercase();
                let name_lower = syn::Ident::new(&name_lower, Span::call_site());
                expanded.extend(quote! {
                    #[allow(unused_macros)]
                    macro_rules! #name_lower {
                        ($(<#($#generic_ident:tt),*>),* $(,)?) => {$(
                            typetag::__private::inventory::submit! {
                                <dyn #object>::typetag_register(
                                    concat!(#name_quotes, "<", #(stringify!($#generic_ident)),*, ">"),
                                    (|deserializer| typetag::__private::Result::Ok(
                                        typetag::__private::Box::new(
                                            typetag::__private::erased_serde::deserialize::<#path<#($#generic_ident),*>>(deserializer)?
                                        ),
                                    )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                                )
                            }
                        )*}
                    }
                    pub(crate) use #name_lower;
                });
            }
        }
    }

    expanded
}

fn augment_impl(input: &mut ItemImpl, generics: &[GenericParam], name: &TokenStream, mode: Mode) {
    if mode.ser {
        // Fill out where clauses
        input.generics.make_where_clause();
        for g in generics {
            if let GenericParam::Type(TypeParam { ident, .. }) = g {
                input
                    .generics
                    .where_clause
                    .as_mut()
                    .unwrap()
                    .predicates
                    .push(parse_quote!(#ident: typetag::Tagged))
            }
        }

        // Fill out extra function definitions
        let mut args = Punctuated::<Expr, Token![,]>::new();
        for g in generics {
            match g {
                GenericParam::Type(TypeParam { ident, .. }) => {
                    args.push(parse_quote!(<#ident as typetag::Tagged>::tag()))
                }
                GenericParam::Const(ConstParam { ident, .. }) => args.push(parse_quote!(#ident)),
                GenericParam::Lifetime(_) => {}
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
