use fxhash::FxHashMap;
use xml::reader::{EventReader, XmlEvent};

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{reserve_player_mut, DatumRef, ScriptError},
};

/// Represents a parsed XML node
#[derive(Clone, Debug)]
pub struct XmlNode {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<XmlNodeChild>,
}

#[derive(Clone, Debug)]
pub enum XmlNodeChild {
    Element(XmlNode),
    Text(String),
}

pub struct XmlParserXtraInstance {
    pub parsed_root: Option<XmlNode>,
    pub error: Option<String>,
    pub ignore_whitespace: bool,
    pub done_parsing: bool,
}

impl XmlParserXtraInstance {
    pub fn new() -> Self {
        XmlParserXtraInstance {
            parsed_root: None,
            error: None,
            ignore_whitespace: false,
            done_parsing: true,
        }
    }

    /// Parse an XML string into the internal node structure
    pub fn parse_string(&mut self, xml_string: &str) -> i32 {
        self.error = None;
        self.parsed_root = None;

        let parser = EventReader::from_str(xml_string);
        let mut stack: Vec<XmlNode> = Vec::new();
        let mut root: Option<XmlNode> = None;

        for event in parser {
            match event {
                Ok(XmlEvent::StartElement { name, attributes, .. }) => {
                    let node = XmlNode {
                        name: name.local_name,
                        attributes: attributes
                            .into_iter()
                            .map(|attr| (attr.name.local_name, attr.value))
                            .collect(),
                        children: Vec::new(),
                    };
                    stack.push(node);
                }
                Ok(XmlEvent::EndElement { .. }) => {
                    if let Some(completed_node) = stack.pop() {
                        if stack.is_empty() {
                            root = Some(completed_node);
                        } else {
                            if let Some(parent) = stack.last_mut() {
                                parent.children.push(XmlNodeChild::Element(completed_node));
                            }
                        }
                    }
                }
                Ok(XmlEvent::Characters(text)) | Ok(XmlEvent::CData(text)) => {
                    let trimmed = if self.ignore_whitespace {
                        text.trim().to_string()
                    } else {
                        text.clone()
                    };

                    // Only add non-empty text nodes (or all if not ignoring whitespace)
                    if !self.ignore_whitespace || !trimmed.is_empty() {
                        if let Some(current) = stack.last_mut() {
                            current.children.push(XmlNodeChild::Text(trimmed));
                        }
                    }
                }
                Ok(XmlEvent::Whitespace(text)) => {
                    if !self.ignore_whitespace {
                        if let Some(current) = stack.last_mut() {
                            current.children.push(XmlNodeChild::Text(text));
                        }
                    }
                }
                Err(e) => {
                    self.error = Some(format!("XML parsing error: {}", e));
                    self.done_parsing = true;
                    return -1;
                }
                _ => {}
            }
        }

        self.parsed_root = root;
        self.done_parsing = true;
        0 // Success
    }

    /// Convert the parsed XML tree to a Lingo property list structure
    fn node_to_prop_list(node: &XmlNode) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Create #name property
            let name_key = player.alloc_datum(Datum::Symbol("name".to_string()));
            let name_value = player.alloc_datum(Datum::String(node.name.clone()));

            // Create #attributes property list
            let attributes_key = player.alloc_datum(Datum::Symbol("attributes".to_string()));
            let mut attr_pairs: Vec<(DatumRef, DatumRef)> = Vec::new();
            for (attr_name, attr_value) in &node.attributes {
                let attr_key = player.alloc_datum(Datum::Symbol(attr_name.clone()));
                let attr_val = player.alloc_datum(Datum::String(attr_value.clone()));
                attr_pairs.push((attr_key, attr_val));
            }
            let attributes_value = player.alloc_datum(Datum::PropList(attr_pairs, false));

            // Create #children list
            let children_key = player.alloc_datum(Datum::Symbol("children".to_string()));
            let mut children_refs: Vec<DatumRef> = Vec::new();

            for child in &node.children {
                match child {
                    XmlNodeChild::Element(child_node) => {
                        // Recursively convert child elements
                        // We need to drop the player borrow before recursing
                        let child_ref = Self::node_to_prop_list_inner(player, child_node)?;
                        children_refs.push(child_ref);
                    }
                    XmlNodeChild::Text(text) => {
                        // Text nodes are added as strings directly to children
                        let text_ref = player.alloc_datum(Datum::String(text.clone()));
                        children_refs.push(text_ref);
                    }
                }
            }

            let children_value =
                player.alloc_datum(Datum::List(DatumType::List, children_refs, false));

            // Build the property list: [#name: "tagname", #attributes: [:], #children: [...]]
            let prop_list = Datum::PropList(
                vec![
                    (name_key, name_value),
                    (attributes_key, attributes_value),
                    (children_key, children_value),
                ],
                false,
            );

            Ok(player.alloc_datum(prop_list))
        })
    }

    /// Inner helper to convert node to prop list when player is already borrowed
    fn node_to_prop_list_inner(
        player: &mut crate::player::DirPlayer,
        node: &XmlNode,
    ) -> Result<DatumRef, ScriptError> {
        // Create #name property
        let name_key = player.alloc_datum(Datum::Symbol("name".to_string()));
        let name_value = player.alloc_datum(Datum::String(node.name.clone()));

        // Create #attributes property list
        let attributes_key = player.alloc_datum(Datum::Symbol("attributes".to_string()));
        let mut attr_pairs: Vec<(DatumRef, DatumRef)> = Vec::new();
        for (attr_name, attr_value) in &node.attributes {
            let attr_key = player.alloc_datum(Datum::Symbol(attr_name.clone()));
            let attr_val = player.alloc_datum(Datum::String(attr_value.clone()));
            attr_pairs.push((attr_key, attr_val));
        }
        let attributes_value = player.alloc_datum(Datum::PropList(attr_pairs, false));

        // Create #children list
        let children_key = player.alloc_datum(Datum::Symbol("children".to_string()));
        let mut children_refs: Vec<DatumRef> = Vec::new();

        for child in &node.children {
            match child {
                XmlNodeChild::Element(child_node) => {
                    let child_ref = Self::node_to_prop_list_inner(player, child_node)?;
                    children_refs.push(child_ref);
                }
                XmlNodeChild::Text(text) => {
                    let text_ref = player.alloc_datum(Datum::String(text.clone()));
                    children_refs.push(text_ref);
                }
            }
        }

        let children_value =
            player.alloc_datum(Datum::List(DatumType::List, children_refs, false));

        // Build the property list: [#name: "tagname", #attributes: [:], #children: [...]]
        let prop_list = Datum::PropList(
            vec![
                (name_key, name_value),
                (attributes_key, attributes_value),
                (children_key, children_value),
            ],
            false,
        );

        Ok(player.alloc_datum(prop_list))
    }

    /// makeList handler - converts parsed XML to Lingo property list
    pub fn make_list(&self) -> Result<DatumRef, ScriptError> {
        if let Some(ref root) = self.parsed_root {
            Self::node_to_prop_list(root)
        } else {
            Ok(DatumRef::Void)
        }
    }

    /// Convert a text node to a prop list with empty name (for getPropRef compatibility)
    fn text_node_to_prop_list(text: &str) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Create #name property (empty string for text nodes)
            let name_key = player.alloc_datum(Datum::Symbol("name".to_string()));
            let name_value = player.alloc_datum(Datum::String(String::new()));

            // Create #attributes property list (empty for text nodes)
            let attributes_key = player.alloc_datum(Datum::Symbol("attributes".to_string()));
            let attributes_value = player.alloc_datum(Datum::PropList(vec![], false));

            // Create #charData property with the text content
            let chardata_key = player.alloc_datum(Datum::Symbol("charData".to_string()));
            let chardata_value = player.alloc_datum(Datum::String(text.to_string()));

            // Create #children list (empty for text nodes)
            let children_key = player.alloc_datum(Datum::Symbol("children".to_string()));
            let children_value =
                player.alloc_datum(Datum::List(DatumType::List, vec![], false));

            // Build the property list
            let prop_list = Datum::PropList(
                vec![
                    (name_key, name_value),
                    (attributes_key, attributes_value),
                    (chardata_key, chardata_value),
                    (children_key, children_value),
                ],
                false,
            );

            Ok(player.alloc_datum(prop_list))
        })
    }
}

pub struct XmlParserXtraManager {
    pub instances: FxHashMap<u32, XmlParserXtraInstance>,
    pub instance_counter: u32,
}

impl XmlParserXtraManager {
    pub fn new() -> Self {
        XmlParserXtraManager {
            instances: FxHashMap::default(),
            instance_counter: 0,
        }
    }

    pub fn create_instance(&mut self, _args: &Vec<DatumRef>) -> u32 {
        self.instance_counter += 1;
        self.instances
            .insert(self.instance_counter, XmlParserXtraInstance::new());
        self.instance_counter
    }

    pub fn has_instance_async_handler(_name: &String) -> bool {
        // parseURL could be async, but we'll implement it synchronously for now
        false
    }

    pub async fn call_instance_async_handler(
        handler_name: &String,
        instance_id: u32,
        _args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        Err(ScriptError::new(format!(
            "No async handler {} found for XmlParser xtra instance #{}",
            handler_name, instance_id
        )))
    }

    pub fn call_instance_handler(
        handler_name: &String,
        instance_id: u32,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let manager = unsafe { XMLPARSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
        let instance = manager.instances.get_mut(&instance_id).ok_or_else(|| {
            ScriptError::new(format!("XmlParser instance #{} not found", instance_id))
        })?;

        match handler_name.to_lowercase().as_str() {
            "parsestring" => {
                let xml_string = crate::player::reserve_player_ref(|player| {
                    let arg = args.get(0).ok_or_else(|| {
                        ScriptError::new("parseString requires a string argument".to_string())
                    })?;
                    player.get_datum(arg).string_value()
                })?;

                let result = instance.parse_string(&xml_string);
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(result))))
            }
            "makelist" => instance.make_list(),
            "makesublist" => {
                // makeSubList returns a property list for the root node (or could be called on child refs)
                instance.make_list()
            }
            "geterror" => {
                if let Some(ref error) = instance.error {
                    reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::String(error.clone())))
                    })
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "ignorewhitespace" => {
                let ignore = crate::player::reserve_player_ref(|player| {
                    let arg = args.get(0).ok_or_else(|| {
                        ScriptError::new(
                            "ignoreWhiteSpace requires a boolean argument".to_string(),
                        )
                    })?;
                    player.get_datum(arg).bool_value()
                })?;

                instance.ignore_whitespace = ignore;
                Ok(DatumRef::Void)
            }
            "doneparsing" => reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Int(if instance.done_parsing { 1 } else { 0 })))
            }),
            "parseurl" => {
                // parseURL is not fully implemented - would require async HTTP fetch
                // For now, return an error suggesting to use getNetText + parseString
                Err(ScriptError::new(
                    "parseURL is not supported. Use getNetText to fetch XML, then parseString."
                        .to_string(),
                ))
            }
            // Manual node traversal methods
            "count" => {
                // count(#child) returns the number of children
                // count(#attribute) returns the number of attributes
                let prop_name = crate::player::reserve_player_ref(|player| {
                    if let Some(arg) = args.get(0) {
                        player.get_datum(arg).symbol_value()
                    } else {
                        Ok("child".to_string()) // Default to child count
                    }
                })?;

                if let Some(ref root) = instance.parsed_root {
                    let count = match prop_name.to_lowercase().as_str() {
                        "child" | "children" => root.children.len() as i32,
                        "attribute" | "attributes" => root.attributes.len() as i32,
                        _ => 0,
                    };
                    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(count))))
                } else {
                    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(0))))
                }
            }
            "child" => {
                // child[n] returns the nth child node
                let index = crate::player::reserve_player_ref(|player| {
                    let arg = args.get(0).ok_or_else(|| {
                        ScriptError::new("child requires an index argument".to_string())
                    })?;
                    player.get_datum(arg).int_value()
                })?;

                if let Some(ref root) = instance.parsed_root {
                    // Lingo uses 1-based indexing
                    let idx = (index - 1) as usize;
                    if idx < root.children.len() {
                        match &root.children[idx] {
                            XmlNodeChild::Element(child_node) => {
                                XmlParserXtraInstance::node_to_prop_list(child_node)
                            }
                            XmlNodeChild::Text(text) => {
                                XmlParserXtraInstance::text_node_to_prop_list(text)
                            }
                        }
                    } else {
                        Ok(DatumRef::Void)
                    }
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "name" => {
                // Returns the tag name of the root element
                if let Some(ref root) = instance.parsed_root {
                    reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::String(root.name.clone())))
                    })
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "attributename" => {
                // attributeName[n] returns the name of the nth attribute
                let index = crate::player::reserve_player_ref(|player| {
                    let arg = args.get(0).ok_or_else(|| {
                        ScriptError::new("attributeName requires an index argument".to_string())
                    })?;
                    player.get_datum(arg).int_value()
                })?;

                if let Some(ref root) = instance.parsed_root {
                    let idx = (index - 1) as usize;
                    if idx < root.attributes.len() {
                        reserve_player_mut(|player| {
                            Ok(player
                                .alloc_datum(Datum::String(root.attributes[idx].0.clone())))
                        })
                    } else {
                        Ok(DatumRef::Void)
                    }
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "attributevalue" => {
                // attributeValue[n] or attributeValue["name"] returns the attribute value
                let (by_index, index, attr_name) = crate::player::reserve_player_ref(|player| {
                    let arg = args.get(0).ok_or_else(|| {
                        ScriptError::new("attributeValue requires an argument".to_string())
                    })?;
                    let datum = player.get_datum(arg);
                    if datum.is_int() || datum.is_number() {
                        Ok((true, datum.int_value()?, String::new()))
                    } else {
                        Ok((false, 0, datum.string_value()?))
                    }
                })?;

                if let Some(ref root) = instance.parsed_root {
                    if by_index {
                        let idx = (index - 1) as usize;
                        if idx < root.attributes.len() {
                            reserve_player_mut(|player| {
                                Ok(player.alloc_datum(Datum::String(
                                    root.attributes[idx].1.clone(),
                                )))
                            })
                        } else {
                            Ok(DatumRef::Void)
                        }
                    } else {
                        // Find attribute by name
                        if let Some((_, value)) =
                            root.attributes.iter().find(|(name, _)| name == &attr_name)
                        {
                            reserve_player_mut(|player| {
                                Ok(player.alloc_datum(Datum::String(value.clone())))
                            })
                        } else {
                            Ok(DatumRef::Void)
                        }
                    }
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "getpropref" => {
                // getPropRef(#child, n) returns a property list for the nth child
                // getPropRef(#attribute, n) returns the nth attribute as [name, value]
                let (prop_name, index) = crate::player::reserve_player_ref(|player| {
                    let prop_arg = args.get(0).ok_or_else(|| {
                        ScriptError::new("getPropRef requires a property name".to_string())
                    })?;
                    let index_arg = args.get(1).ok_or_else(|| {
                        ScriptError::new("getPropRef requires an index".to_string())
                    })?;
                    let prop_name = player.get_datum(prop_arg).symbol_value()?;
                    let index = player.get_datum(index_arg).int_value()?;
                    Ok((prop_name, index))
                })?;

                if let Some(ref root) = instance.parsed_root {
                    match prop_name.to_lowercase().as_str() {
                        "child" | "children" => {
                            let idx = (index - 1) as usize;
                            if idx < root.children.len() {
                                match &root.children[idx] {
                                    XmlNodeChild::Element(child_node) => {
                                        XmlParserXtraInstance::node_to_prop_list(child_node)
                                    }
                                    XmlNodeChild::Text(text) => {
                                        // Wrap text nodes in a PropList with #charData for compatibility
                                        // This allows .name access to return empty string instead of error
                                        XmlParserXtraInstance::text_node_to_prop_list(text)
                                    }
                                }
                            } else {
                                Ok(DatumRef::Void)
                            }
                        }
                        "attribute" | "attributes" => {
                            let idx = (index - 1) as usize;
                            if idx < root.attributes.len() {
                                reserve_player_mut(|player| {
                                    let (name, value) = &root.attributes[idx];
                                    let name_ref =
                                        player.alloc_datum(Datum::String(name.clone()));
                                    let value_ref =
                                        player.alloc_datum(Datum::String(value.clone()));
                                    Ok(player.alloc_datum(Datum::List(
                                        DatumType::List,
                                        vec![name_ref, value_ref],
                                        false,
                                    )))
                                })
                            } else {
                                Ok(DatumRef::Void)
                            }
                        }
                        _ => Ok(DatumRef::Void),
                    }
                } else {
                    Ok(DatumRef::Void)
                }
            }
            _ => Err(ScriptError::new(format!(
                "No handler {} found for XmlParser xtra instance #{}",
                handler_name, instance_id
            ))),
        }
    }
}

pub fn borrow_xmlparser_manager_mut<T>(
    callback: impl FnOnce(&mut XmlParserXtraManager) -> T,
) -> T {
    let manager = unsafe { XMLPARSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
    callback(manager)
}

pub static mut XMLPARSER_XTRA_MANAGER_OPT: Option<XmlParserXtraManager> = None;
