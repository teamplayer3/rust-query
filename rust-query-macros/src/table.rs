use super::make_generic;
use heck::ToSnekCase;
use quote::{format_ident, quote};

use proc_macro2::TokenStream;

use syn::Ident;

use super::Table;

pub(crate) fn define_table(table: &Table, schema: &Ident) -> TokenStream {
    let table_ident = &table.name;
    let columns = &table.columns;
    let uniques = &table.uniques;

    let mut defs = vec![];
    let mut typs = vec![];
    let mut typ_asserts = vec![];
    let mut generics = vec![];
    let mut read_bounds = vec![];
    let mut inits = vec![];
    let mut reads = vec![];
    let mut def_typs = vec![];

    for col in columns.values() {
        let typ = &col.typ;
        let ident = &col.name;
        let ident_str = ident.to_string();
        let generic = make_generic(ident);
        defs.push(quote!(pub #ident: #generic));
        typs.push(quote!(::rust_query::Db<'t, #typ>));
        typ_asserts.push(quote!(::rust_query::valid_in_schema::<#schema, #typ>();));
        read_bounds.push(quote!(#generic: ::rust_query::Value<'t, Typ=#typ>));
        generics.push(generic);
        inits.push(quote!(#ident: f.col(#ident_str)));
        reads.push(quote!(f.col(#ident_str, self.#ident)));
        def_typs.push(quote!(f.col::<#typ>(#ident_str)))
    }

    let dummy_ident = format_ident!("{}Dummy", table_ident);

    let table_name: &String = &table_ident.to_string().to_snek_case();
    let has_id = quote!(
        impl ::rust_query::HasId for #table_ident {
            const ID: &'static str = "id";
            const NAME: &'static str = #table_name;
        }
    );

    quote! {
        pub struct #table_ident(());

        pub struct #dummy_ident<#(#generics),*> {
            #(#defs,)*
        }

        impl ::rust_query::Table for #table_ident {
            type Dummy<'t> = #dummy_ident<#(#typs),*>;
            type Schema = #schema;

            fn name(&self) -> String {
                #table_name.to_owned()
            }

            fn build(f: ::rust_query::Builder<'_>) -> Self::Dummy<'_> {
                #dummy_ident {
                    #(#inits,)*
                }
            }

            fn typs(f: &mut ::rust_query::TypBuilder) {
                #(#def_typs;)*
                #(#uniques;)*
            }
        }

        impl<'t, #(#read_bounds),*> ::rust_query::private::Writable<'t> for #dummy_ident<#(#generics),*> {
            type T = #table_ident;
            fn read(self: Box<Self>, f: ::rust_query::private::Reader<'_, 't>) {
                #(#reads;)*
            }
        }

        const _: fn() = || {
            #(#typ_asserts)*
        };

        #has_id
    }
}
