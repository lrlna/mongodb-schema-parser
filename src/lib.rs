//! #Infer a probabilistic schema for a MongoDB collection.
//! This crate creates a probabilistic scehma given a json-style string
//! representing a MongoDB collection. It can be used in both rust and javascript
//! given a WASM compilation.
//!
//! ## Usage: in Rust
//! ```rust
//! use mongodb_schema_parser::SchemaParser;
//!
//! pub fn main () {
//!   let mut file = fs::read_to_string("examples/fanclub.json").unwrap();
//!   let file: Vec<&str> = file.split("\n").collect();
//!   let schema_parser = SchemaParser::new();
//!   for json in file {
//!     schema_parser.write(&json)?;
//!   }
//!   let result = schema_parser.to_json();
//! }
//! ```
//!
//! ## Usage: in JavaScript
//! Make sure your environment is setup for Web Assembly usage.
//! ```js
//! import { SchemaParser } from "mongodb-schema-parser";
//!
//! const schemaParser = new SchemaParser()
//!
//! // get the json file
//! fetch('./fanclub.json')
//!   .then(response => response.text())
//!   .then(data => {
//!     var json = data.split("\n")
//!     for (var i = 0; i < json.length; i++) {
//!       if (json[i] !== '') {
//!         // feed the parser json line by line
//!         schemaParser.write(json[i])
//!       }
//!     }
//!     // get the result as a json string
//!     var result = schemaParser.toJson()
//!     console.log(result)
//!   })
//! ```

#![cfg_attr(feature = "nightly", deny(missing_docs))]
#![cfg_attr(feature = "nightly", feature(external_doc))]
#![cfg_attr(feature = "nightly", doc(include = "../README.md"))]
#![cfg_attr(feature = "nightly", deny(unsafe_code))]
#![allow(clippy::new_without_default_derive)]
//#![cfg_attr(test, deny(warnings))]

extern crate bson;
use bson::{Bson, Document};

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;
use serde_json::Value;

use wasm_bindgen::prelude::*;

// using custom allocator which is built specifically for wasm; makes it smaller
// + faster
extern crate wee_alloc;
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

use std::mem;
use std::string::String;

mod field;
use crate::field::Field;

mod field_type;
use crate::field_type::FieldType;

mod value_type;
use crate::value_type::ValueType;

extern crate failure;

#[wasm_bindgen]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SchemaParser {
  count: i64,
  fields: Vec<Field>,
}

// Need to wrap schema parser impl for wasm suppport.
// Here we are wrapping the exported to JS land methods and mathing on Result to
// turn the error message to JsValue.
#[wasm_bindgen]
impl SchemaParser {
  /// Wrapper method for `SchemaParser::new()` to be used in JavaScript.
  /// `wasm_bindgen(js_name = "new")`
  #[wasm_bindgen(constructor)]
  #[wasm_bindgen(js_name = "new")]
  pub fn wasm_new() -> Self {
    Self::new()
  }

  /// Wrapper method for `schema_parser.write()` to be used in JavaScript.
  /// `wasm_bindgen(js_name = "write")`
  #[wasm_bindgen(js_name = "write")]
  pub fn wasm_write(&mut self, json: &str) -> Result<(), JsValue> {
    match self.write(json) {
      Err(e) => Err(JsValue::from_str(&format!("{}", e))),
      _ => Ok(()),
    }
  }

  /// Wrapper method for `schema_parser.to_json()` to be used in JavaScript.
  /// `wasm_bindgen(js_name = "toJson")`
  #[wasm_bindgen(js_name = "toJson")]
  pub fn wasm_to_json(&mut self) -> Result<String, JsValue> {
    match self.to_json() {
      Err(e) => Err(JsValue::from_str(&format!("{}", e))),
      Ok(val) => Ok(val),
    }
  }
}

impl SchemaParser {
  /// Returns a new instance of Schema Parser populated with zero `count` and an
  /// empty `fields` vector.
  ///
  /// # Examples
  /// ```
  /// use mongodb_schema_parser::SchemaParser;
  /// let schema_parser = SchemaParser::new();
  /// ```
  #[inline]
  pub fn new() -> Self {
    SchemaParser {
      count: 0,
      fields: Vec::new(),
    }
  }

  /// Writes json-like string slices SchemaParser's fields vector.
  ///
  /// # Arguments
  /// * `json` - A json-like string slice. i.e { "name": "Nori", "type": "Cat"}
  ///
  /// # Examples
  /// ```
  /// use mongodb_schema_parser::SchemaParser;
  /// let schema_parser = SchemaParser::new();
  /// let json = "{ "name": "Chashu", "type": "Cat" }";
  /// schema_parser.write(&json);
  /// ```
  #[inline]
  pub fn write(
    &mut self,
    json: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let val: Value = serde_json::from_str(json)?;
    let bson = Bson::from(val);
    // should do a match for NoneError
    let doc = bson.as_document().unwrap().to_owned();
    let count = &self.count + 1;
    mem::replace(&mut self.count, count);
    self.generate_field(doc, &None);
    Ok(())
  }

  /// Returns a serde_json string. This should be called after all values were
  /// written. This is also the result of the parsed documents.
  ///
  /// # Examples
  /// ```
  /// use mongodb_schema_parser::SchemaParser;
  /// let schema_parser = SchemaParser::new();
  /// let json = "{ "name": "Chashu", "type": "Cat" }";
  /// schema_parser.write(&json);
  /// let schema = schema_parser.to_json().unwrap();
  /// println!("{}")
  /// ```
  #[inline]
  pub fn to_json(
    &self,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(serde_json::to_string(&self)?)
  }

  #[inline]
  fn add_to_fields(&mut self, field: Field) {
    self.fields.push(field)
  }

  // why do i have to explicitly return true instead of just returning field.name == key
  #[inline]
  fn does_field_name_exist(&mut self, key: &str) -> bool {
    for field in &mut self.fields {
      if field.name == key {
        return true;
      }
    }
    false
  }

  #[inline]
  fn update_field(&mut self, key: &str, value: &Bson) {
    // need to set count here as well
    // maybe store the names in a hash map so then it's easier to look up the key
    for field in &mut self.fields {
      if field.name == key {
        for field_type in &mut field.types {
          // update field type,
          field_type.update_count();
          field_type.update_value(&value);
        }
      }
    }
  }

  #[inline]
  fn generate_field(&mut self, doc: Document, path: &Option<String>) {
    let count = 0;

    for (key, value) in doc {
      // check if we already have a field for this key;
      // this check should also be checking for uniqueness
      // 'inner:
      // if name exist, call self.update_field -- should iterate over itself and call update field
      if self.does_field_name_exist(&key) {
        self.update_field(&key, &value);
      } else {
        // if name doesn't exist, proceed by this path and create a new field
        let current_path = Field::get_path(key.clone(), path);
        let mut field = Field::new(&key, &current_path, count);

        match &value {
          Bson::Document(subdoc) => {
            self.generate_field(subdoc.to_owned(), &Some(current_path));
          }
          _ => {
            let field_type = FieldType::new(&current_path).add_to_type(&value);
            field.add_to_types(field_type.to_owned());
          }
        };
        self.add_to_fields(field);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  // use super::*;

  #[test]
  fn it_creates_new() {}

  #[test]
  fn it_writes() {}

  #[test]
  fn it_formats_to_json() {}

  #[test]
  fn it_adds_to_fields() {}

  #[test]
  fn it_checks_if_field_name_exists() {}

  #[test]
  fn it_updates_fields() {}

  #[test]
  fn it_generates_fields() {}
}
