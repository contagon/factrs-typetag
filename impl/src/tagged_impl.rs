use crate::{ImplArgs, Mode};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{parse_quote, Error, ItemImpl, PathArguments, Type, TypePath};

pub(crate) fn expand(args: ImplArgs, mut input: ItemImpl, mode: Mode) -> TokenStream {
    let name = match args.name {
        Some(name) => quote!(#name),
        None => match type_name(&input.self_ty) {
            Some(name) => quote!(#name),
            None => {
                let msg = "use #[typetag::serde(name = \"...\")] to specify a unique name";
                return Error::new_spanned(&input.self_ty, msg).to_compile_error();
            }
        },
    };

    let mut name_lower = name.to_string().to_lowercase();
    name_lower.pop();
    name_lower.remove(0);
    let name_lower = syn::Ident::new(&name_lower, Span::call_site());
    let name_lower = quote!(#name_lower);

    augment_impl(&mut input, &name, mode);

    let object = &input.trait_.as_ref().unwrap().1;
    let this = &input.self_ty;

    let mut expanded = quote! {
        #input
    };

    if mode.de {
        if input.generics.params.is_empty() {
            expanded.extend(quote! {
                typetag::__private::inventory::submit! {
                    <dyn #object>::typetag_register(
                        #name,
                        (|deserializer| typetag::__private::Result::Ok(
                            typetag::__private::Box::new(
                                typetag::__private::erased_serde::deserialize::<#this>(deserializer)?
                            ),
                        )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                    )
                }
            });
        } else {
            // Get name of type without generics
            let mut path = match this.as_ref() {
                Type::Path(path) => path.path.clone(),
                _ => {
                    return Error::new_spanned(&this, "Expected path for generic version")
                        .to_compile_error()
                }
            };
            path.segments.last_mut().unwrap().arguments = PathArguments::None;

            // Add in macro
            expanded.extend(quote! {
                mod tag {
                    #[allow(unused_macros)]
                    macro_rules! #name_lower {
                        (<$($generic:ident),*>) => {
                            impl typetag::Tag for #path< $($generic),*> {
                                fn typetag_name(&self) -> String {
                                    String::from(concat!(#name, "<", $(stringify!($generic)),*, ">"))
                                }
                            }
                            typetag::__private::inventory::submit! {
                                <dyn PlainTrait>::typetag_register(
                                    concat!(#name, "<", $(stringify!($generic)),*, ">"),
                                    (|deserializer| typetag::__private::Result::Ok(
                                        typetag::__private::Box::new(
                                            typetag::__private::erased_serde::deserialize::<#path<$($generic),*>>(deserializer)?
                                        ),
                                    )) as typetag::__private::DeserializeFn<<dyn PlainTrait as typetag::__private::Strictest>::Object>,
                                )
                            }
                        }
                    }
                    pub(crate) use #name_lower;
                }
            });
            // println!("expanded: {}", expanded);
        }
    }

    expanded
}

fn augment_impl(input: &mut ItemImpl, name: &TokenStream, mode: Mode) {
    // If there's generics we need to handle separately
    if !input.generics.params.is_empty() && mode.de {
        if mode.ser {
            match &mut input.generics.where_clause {
                Some(ref mut where_clause) => where_clause
                    .predicates
                    .push(parse_quote!(Self: typetag::Tag)),
                None => input.generics.where_clause = Some(parse_quote!(where Self: typetag::Tag)),
            }

            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    <Self as typetag::Tag>::typetag_name(self)
                }
            });
        }

        input.items.push(parse_quote! {
            #[doc(hidden)]
            fn typetag_deserialize(&self) {}
        });

    // Otherwise just handle things regularly
    } else {
        if mode.ser {
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    String::from(#name)
                }
            });
        }
        if mode.de {
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_deserialize(&self) {}
            });
        }
    }
}

fn type_name(mut ty: &Type) -> Option<String> {
    loop {
        match ty {
            Type::Path(TypePath { qself: None, path }) => {
                return Some(path.segments.last().unwrap().ident.to_string());
            }
            Type::Group(group) => {
                ty = &group.elem;
            }
            _ => return None,
        }
    }
}
