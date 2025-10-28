use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};
use std::collections::HashMap;
use std::io::Cursor;
use xml::attribute::OwnedAttribute;
use xml::reader::{EventReader, XmlEvent};

#[derive(Debug, Clone)]
pub struct XmlDocument {
    pub id: u32,
    pub root_element: Option<u32>,
    pub content: String,
    pub ignore_white: bool,
}

#[derive(Debug, Clone)]
pub struct XmlNode {
    pub id: u32,
    pub node_type: XmlNodeType,
    pub name: String,
    pub value: Option<String>,
    pub attributes: HashMap<String, String>,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum XmlNodeType {
    Document,
    Element,
    Text,
    Comment,
    ProcessingInstruction,
    CData,
}

pub struct XmlParser {
    next_xml_id: u32,
}

impl XmlParser {
    pub fn new(start_id: u32) -> Self {
        Self {
            next_xml_id: start_id,
        }
    }

    // Parse XML content using xml-rs and create node structure
    pub fn parse_xml_content(
        &mut self,
        content: &str,
    ) -> Result<(Option<u32>, HashMap<u32, XmlNode>), ScriptError> {
        if content.trim().is_empty() {
            return Ok((None, HashMap::new()));
        }

        let cursor = Cursor::new(content.as_bytes());
        let parser = EventReader::new(cursor);

        let mut nodes: HashMap<u32, XmlNode> = HashMap::new();
        let mut element_stack: Vec<u32> = Vec::new();
        let mut root_element: Option<u32> = None;
        let mut current_text = String::new();

        for event in parser {
            match event {
                Ok(XmlEvent::StartElement {
                    name, attributes, ..
                }) => {
                    let element_id = self.next_xml_id;
                    self.next_xml_id += 1;

                    let attr_map = Self::convert_attributes(attributes);

                    let element_node = XmlNode {
                        id: element_id,
                        node_type: XmlNodeType::Element,
                        name: name.local_name.to_lowercase(),
                        value: None,
                        attributes: attr_map.clone(),
                        parent: element_stack.last().copied(),
                        children: Vec::new(),
                    };

                    if let Some(&parent_id) = element_stack.last() {
                        if let Some(parent_node) = nodes.get_mut(&parent_id) {
                            parent_node.children.push(element_id);
                        }
                    }

                    nodes.insert(element_id, element_node);

                    if root_element.is_none() {
                        root_element = Some(element_id);
                    }

                    element_stack.push(element_id);
                }

                Ok(XmlEvent::EndElement { .. }) => {
                    if !current_text.trim().is_empty() {
                        if let Some(&parent_id) = element_stack.last() {
                            self.add_text_node(&mut nodes, parent_id, current_text.trim());
                        }
                        current_text.clear();
                    }
                    element_stack.pop();
                }

                Ok(XmlEvent::Characters(text)) => {
                    current_text.push_str(&text);
                }

                Ok(XmlEvent::CData(cdata)) => {
                    if let Some(&parent_id) = element_stack.last() {
                        self.add_cdata_node(&mut nodes, parent_id, &cdata);
                    }
                }

                Ok(XmlEvent::Comment(comment)) => {
                    if let Some(&parent_id) = element_stack.last() {
                        self.add_comment_node(&mut nodes, parent_id, &comment);
                    }
                }

                Ok(XmlEvent::ProcessingInstruction { name, data }) => {
                    if let Some(&parent_id) = element_stack.last() {
                        self.add_processing_instruction_node(
                            &mut nodes,
                            parent_id,
                            &name,
                            data.as_deref(),
                        );
                    }
                }

                Ok(XmlEvent::Whitespace(_)) => {
                    // Handle whitespace based on ignoreWhite setting
                }

                Ok(XmlEvent::StartDocument { .. }) => {
                    web_sys::console::log_1(&"🔧 XML document parsing started".into());
                }

                Ok(XmlEvent::EndDocument) => {
                    web_sys::console::log_1(&"🔧 XML document parsing completed".into());
                }

                Ok(XmlEvent::Doctype { .. }) => {
                    // Ignore DOCTYPE declarations
                    web_sys::console::log_1(&"🔧 Skipping DOCTYPE declaration".into());
                }

                Err(e) => {
                    web_sys::console::log_1(&format!("🔧 ❌ XML parsing error: {}", e).into());
                    return Err(ScriptError::new(format!("XML parsing error: {}", e)));
                }
            }
        }

        Ok((root_element, nodes))
    }

    fn convert_attributes(attributes: Vec<OwnedAttribute>) -> HashMap<String, String> {
        let mut attr_map = HashMap::new();
        for attr in attributes {
            // normalize the key for case-insensitive Lingo lookup
            attr_map.insert(
                attr.name.local_name.to_lowercase(),
                attr.value.trim_matches('"').to_string(),
            );
        }
        attr_map
    }

    fn add_text_node(&mut self, nodes: &mut HashMap<u32, XmlNode>, parent_id: u32, text: &str) {
        let text_id = self.next_xml_id;
        self.next_xml_id += 1;

        let text_node = XmlNode {
            id: text_id,
            node_type: XmlNodeType::Text,
            name: "#text".to_string(),
            value: Some(text.to_string()),
            attributes: HashMap::new(),
            parent: Some(parent_id),
            children: Vec::new(),
        };

        if let Some(parent) = nodes.get_mut(&parent_id) {
            parent.children.push(text_id);
        }

        nodes.insert(text_id, text_node);
        web_sys::console::log_1(
            &format!("🔧 Created text node: '{}' (ID: {})", text, text_id).into(),
        );
    }

    fn add_cdata_node(&mut self, nodes: &mut HashMap<u32, XmlNode>, parent_id: u32, cdata: &str) {
        let cdata_id = self.next_xml_id;
        self.next_xml_id += 1;

        let cdata_node = XmlNode {
            id: cdata_id,
            node_type: XmlNodeType::CData,
            name: "#cdata-section".to_string(),
            value: Some(cdata.to_string()),
            attributes: HashMap::new(),
            parent: Some(parent_id),
            children: Vec::new(),
        };

        if let Some(parent) = nodes.get_mut(&parent_id) {
            parent.children.push(cdata_id);
        }

        nodes.insert(cdata_id, cdata_node);
    }

    fn add_comment_node(
        &mut self,
        nodes: &mut HashMap<u32, XmlNode>,
        parent_id: u32,
        comment: &str,
    ) {
        let comment_id = self.next_xml_id;
        self.next_xml_id += 1;

        let comment_node = XmlNode {
            id: comment_id,
            node_type: XmlNodeType::Comment,
            name: "#comment".to_string(),
            value: Some(comment.to_string()),
            attributes: HashMap::new(),
            parent: Some(parent_id),
            children: Vec::new(),
        };

        if let Some(parent) = nodes.get_mut(&parent_id) {
            parent.children.push(comment_id);
        }

        nodes.insert(comment_id, comment_node);
    }

    fn add_processing_instruction_node(
        &mut self,
        nodes: &mut HashMap<u32, XmlNode>,
        parent_id: u32,
        name: &str,
        data: Option<&str>,
    ) {
        let pi_id = self.next_xml_id;
        self.next_xml_id += 1;

        let pi_node = XmlNode {
            id: pi_id,
            node_type: XmlNodeType::ProcessingInstruction,
            name: name.to_string(),
            value: data.map(|d| d.to_string()),
            attributes: HashMap::new(),
            parent: Some(parent_id),
            children: Vec::new(),
        };

        if let Some(parent) = nodes.get_mut(&parent_id) {
            parent.children.push(pi_id);
        }

        nodes.insert(pi_id, pi_node);
    }
}

// XML Helper Functions
pub struct XmlHelper;

impl XmlHelper {
    // Get children of an XML node (filtered by ignore_white setting)
    pub fn get_node_children(player: &mut DirPlayer, node_id: u32) -> Vec<DatumRef> {
        // Collect child IDs first (immutable borrow only)
        let child_ids: Vec<u32> = if let Some(node) = player.xml_nodes.get(&node_id) {
            node.children.clone()
        } else {
            return Vec::new();
        };

        let mut children = Vec::new();

        for child_id in child_ids {
            if let Some(child_node) = player.xml_nodes.get(&child_id) {
                let should_include = match child_node.node_type {
                    XmlNodeType::Text => {
                        if let Some(text) = &child_node.value {
                            !(text.trim().is_empty()
                                && Self::should_ignore_whitespace(player, node_id))
                        } else {
                            true
                        }
                    }
                    _ => true,
                };

                if should_include {
                    // Now safe: immutable borrow is over, mutable borrow is allowed
                    children.push(player.alloc_datum(Datum::XmlRef(child_id)));
                }
            }
        }

        children
    }

    fn should_ignore_whitespace(player: &mut DirPlayer, node_id: u32) -> bool {
        // Collect doc IDs first (avoids holding an immutable borrow)
        let doc_ids: Vec<u32> = player.xml_documents.keys().cloned().collect();

        for doc_id in doc_ids {
            if Self::node_belongs_to_document(player, node_id, doc_id) {
                // Now borrow doc again immutably, safely
                if let Some(doc) = player.xml_documents.get(&doc_id) {
                    return doc.ignore_white;
                }
            }
        }
        false
    }

    fn node_belongs_to_document(player: &mut DirPlayer, node_id: u32, doc_id: u32) -> bool {
        if let Some(doc) = player.xml_documents.get(&doc_id) {
            if let Some(root_id) = doc.root_element {
                return Self::is_descendant_of(player, node_id, root_id) || node_id == root_id;
            }
        }
        false
    }

    fn is_descendant_of(player: &mut DirPlayer, node_id: u32, ancestor_id: u32) -> bool {
        if let Some(node) = player.xml_nodes.get(&node_id) {
            if let Some(parent_id) = node.parent {
                if parent_id == ancestor_id {
                    return true;
                } else {
                    return Self::is_descendant_of(player, parent_id, ancestor_id);
                }
            }
        }
        false
    }

    pub fn get_node_type_name(player: &mut DirPlayer, node_id: u32) -> String {
        if let Some(node) = player.xml_nodes.get(&node_id) {
            match node.node_type {
                XmlNodeType::Document => "Document".to_string(),
                XmlNodeType::Element => "Element".to_string(),
                XmlNodeType::Text => "Text".to_string(),
                XmlNodeType::Comment => "Comment".to_string(),
                XmlNodeType::ProcessingInstruction => "ProcessingInstruction".to_string(),
                XmlNodeType::CData => "CData".to_string(),
            }
        } else {
            "Unknown".to_string()
        }
    }

    /// Find all nodes with a specific name, searching recursively from start_node_id
    pub fn find_nodes_by_name(
        player: &mut DirPlayer,
        start_node_id: u32,
        target_name: &str,
    ) -> Vec<DatumRef> {
        let mut results = Vec::new();

        // Check if the start node is a document, if so get the root element
        let start_id = if let Some(doc) = player.xml_documents.get(&start_node_id) {
            if let Some(root_id) = doc.root_element {
                root_id
            } else {
                return results;
            }
        } else {
            start_node_id
        };

        // Recursively search for matching nodes
        Self::search_nodes_recursive(player, start_id, target_name, &mut results);

        results
    }

    fn search_nodes_recursive(
        player: &mut DirPlayer,
        node_id: u32,
        target_name: &str,
        results: &mut Vec<DatumRef>,
    ) {
        // Take out what we need from the node first
        let (name, children): (String, Vec<u32>) =
            if let Some(node) = player.xml_nodes.get(&node_id) {
                (node.name.clone(), node.children.clone())
            } else {
                return;
            };

        // Now the immutable borrow is over, safe to do mutable work
        if name == target_name {
            results.push(player.alloc_datum(Datum::XmlRef(node_id)));
        }

        // Recurse into children
        for child_id in children {
            Self::search_nodes_recursive(player, child_id, target_name, results);
        }
    }
}

pub struct XmlDatumHandlers {}

impl XmlDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "parseXML" => Self::parse_xml(datum, args),
            "createElement" => Self::create_element(datum, args),
            "appendChild" => Self::append_child(datum, args),
            "toString" => Self::to_string(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for XML object"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        let xml_id = match player.get_datum(datum) {
            Datum::XmlRef(id) => *id,
            _ => {
                web_sys::console::log_1(&"🔧 ❌ XML get_prop called on non-XML datum".into());
                return Err(ScriptError::new("Invalid XML reference".to_string()));
            }
        };

        Self::get_xml_property(player, xml_id, prop)
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        let xml_id = match player.get_datum(datum) {
            Datum::XmlRef(id) => *id,
            _ => {
                return Err(ScriptError::new("Invalid XML reference".to_string()));
            }
        };

        Self::set_xml_property(player, xml_id, prop, value)
    }

    fn parse_xml(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new(
                "parseXML requires XML string argument".to_string(),
            ));
        }

        reserve_player_mut(|player| {
            // Get the XML parser object ID from datum
            let parser_id = match player.get_datum(datum) {
                Datum::XmlRef(id) => *id,
                _ => {
                    return Err(ScriptError::new(
                        "parseXML must be called on XML parser object".to_string(),
                    ));
                }
            };

            web_sys::console::log_1(
                &format!("🔧 parseXML called on parser object ID {}", parser_id).into(),
            );

            // Check what type the argument is
            let arg_datum = player.get_datum(&args[0]);
            let datum_type = Self::get_datum_type_name(arg_datum);
            web_sys::console::log_1(&format!("🔧 parseXML arg type: {}", datum_type).into());

            let xml_string = match arg_datum {
                Datum::String(s) => s.clone(),
                Datum::Void => {
                    return Err(ScriptError::new(
                        "parseXML argument is Void - expected XML string".to_string(),
                    ));
                }
                _ => {
                    return Err(ScriptError::new(format!(
                        "parseXML requires string argument, got: {}",
                        datum_type
                    )));
                }
            };

            web_sys::console::log_1(
                &format!(
                    "🔧 XML.parseXML called with {} characters",
                    xml_string.len()
                )
                .into(),
            );

            // Instead of creating a NEW document, update the PARSER object's document
            // Parse the XML content
            let mut parser = XmlParser::new(player.next_xml_id);
            let (root_id, nodes) = parser.parse_xml_content(&xml_string)?;

            web_sys::console::log_1(
                &format!(
                    "🔧 Parsing complete. root_id={:?}, parsed {} nodes",
                    root_id,
                    nodes.len()
                )
                .into(),
            );

            // Update the next_xml_id in player
            player.next_xml_id = parser.next_xml_id;

            // Add all parsed nodes to the player's xml_nodes
            for (node_id, node) in nodes {
                player.xml_nodes.insert(node_id, node);
            }

            // Update the parser object (parser_id) with the parsed content
            if let Some(parser_doc) = player.xml_documents.get_mut(&parser_id) {
                parser_doc.root_element = root_id;
                parser_doc.content = xml_string;
                web_sys::console::log_1(
                    &format!(
                        "🔧 Updated parser object {} with root_element={:?}",
                        parser_id, root_id
                    )
                    .into(),
                );
            } else {
                // If the parser object doesn't exist as a document yet, create it
                let xml_doc = XmlDocument {
                    id: parser_id,
                    root_element: root_id,
                    content: xml_string,
                    ignore_white: false,
                };
                player.xml_documents.insert(parser_id, xml_doc);
                web_sys::console::log_1(
                    &format!(
                        "🔧 Created document for parser object {} with root_element={:?}",
                        parser_id, root_id
                    )
                    .into(),
                );
            }

            web_sys::console::log_1(&"🔧 XML parsing completed successfully".into());

            // parseXML returns Void in Lingo
            Ok(DatumRef::Void)
        })
    }

    fn create_element(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new(
                "createElement requires element name argument".to_string(),
            ));
        }

        reserve_player_mut(|player| {
            // Get the document/parser object ID
            let doc_id = match player.get_datum(datum) {
                Datum::XmlRef(id) => *id,
                _ => {
                    return Err(ScriptError::new(
                        "createElement must be called on XML document".to_string(),
                    ));
                }
            };

            // Get the element name from arguments
            let element_name = match player.get_datum(&args[0]) {
                Datum::String(s) => s.clone(),
                Datum::Symbol(s) => s.clone(),
                _ => {
                    return Err(ScriptError::new(
                        "createElement requires string element name".to_string(),
                    ));
                }
            };

            web_sys::console::log_1(
                &format!(
                    "🔧 createElement('{}') called on doc {}",
                    element_name, doc_id
                )
                .into(),
            );

            // Create a new XML element node
            let element_id = player.next_xml_id;
            player.next_xml_id += 1;

            let element_node = XmlNode {
                id: element_id,
                node_type: XmlNodeType::Element,
                name: element_name.clone(),
                value: None,
                attributes: HashMap::new(),
                parent: None, // Not attached to any parent yet
                children: Vec::new(),
            };

            player.xml_nodes.insert(element_id, element_node);

            web_sys::console::log_1(
                &format!(
                    "🔧 Created element '{}' with ID {}",
                    element_name, element_id
                )
                .into(),
            );

            Ok(player.alloc_datum(Datum::XmlRef(element_id)))
        })
    }

    fn append_child(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new(
                "appendChild requires child node argument".to_string(),
            ));
        }

        reserve_player_mut(|player| {
            // Get parent node/document ID
            let parent_id = match player.get_datum(datum) {
                Datum::XmlRef(id) => *id,
                _ => {
                    return Err(ScriptError::new(
                        "appendChild must be called on XML node".to_string(),
                    ));
                }
            };

            // Get child node ID
            let child_id = match player.get_datum(&args[0]) {
                Datum::XmlRef(id) => *id,
                _ => {
                    return Err(ScriptError::new(
                        "appendChild requires XML node argument".to_string(),
                    ));
                }
            };

            web_sys::console::log_1(
                &format!("🔧 appendChild: parent={}, child={}", parent_id, child_id).into(),
            );

            // Check if parent is a document
            if let Some(doc) = player.xml_documents.get_mut(&parent_id) {
                // Appending to document - set as root element
                doc.root_element = Some(child_id);
                web_sys::console::log_1(
                    &format!(
                        "🔧 Set child {} as root of document {}",
                        child_id, parent_id
                    )
                    .into(),
                );
            } else if let Some(_parent_node) = player.xml_nodes.get(&parent_id) {
                // Appending to a node - add to children list
                if let Some(parent_node) = player.xml_nodes.get_mut(&parent_id) {
                    if !parent_node.children.contains(&child_id) {
                        parent_node.children.push(child_id);
                    }
                }

                // Update child's parent reference
                if let Some(child_node) = player.xml_nodes.get_mut(&child_id) {
                    child_node.parent = Some(parent_id);
                }

                web_sys::console::log_1(
                    &format!("🔧 Added child {} to node {}", child_id, parent_id).into(),
                );
            } else {
                return Err(ScriptError::new(format!(
                    "Parent node {} not found",
                    parent_id
                )));
            }

            // Return the child node (Director appendChild returns the appended child)
            Ok(args[0].clone())
        })
    }

    // Create a new XML document and return its ID
    fn create_xml_document(player: &mut DirPlayer, content: String) -> Result<u32, ScriptError> {
        let doc_id = player.next_xml_id;
        player.next_xml_id += 1;

        web_sys::console::log_1(&format!("🔧 Creating XML document with ID {}", doc_id).into());
        web_sys::console::log_1(&format!("🔧 Content length: {} bytes", content.len()).into());

        let mut parser = XmlParser::new(player.next_xml_id);
        let (root_id, nodes) = parser.parse_xml_content(&content)?;

        web_sys::console::log_1(
            &format!(
                "🔧 Parsing complete. root_id={:?}, parsed {} nodes",
                root_id,
                nodes.len()
            )
            .into(),
        );

        // Update the next_xml_id in player
        player.next_xml_id = parser.next_xml_id;

        // Add all parsed nodes to the player's xml_nodes
        for (node_id, node) in nodes {
            web_sys::console::log_1(&format!("🔧 Storing node {} ({})", node_id, node.name).into());
            player.xml_nodes.insert(node_id, node);
        }

        let xml_doc = XmlDocument {
            id: doc_id,
            root_element: root_id,
            content,
            ignore_white: false,
        };

        web_sys::console::log_1(
            &format!(
                "🔧 Storing document {} with root_element={:?}",
                doc_id, root_id
            )
            .into(),
        );
        player.xml_documents.insert(doc_id, xml_doc);

        // Verify storage
        if player.xml_documents.contains_key(&doc_id) {
            web_sys::console::log_1(
                &format!("🔧 ✓ Document {} stored successfully", doc_id).into(),
            );
        } else {
            web_sys::console::log_1(&format!("🔧 ❌ FAILED to store document {}!", doc_id).into());
        }

        Ok(doc_id)
    }

    fn to_string(datum: &DatumRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let xml_id = match player.get_datum(datum) {
                Datum::XmlRef(id) => *id,
                _ => {
                    return Err(ScriptError::new(
                        "toString must be called on XML object".to_string(),
                    ));
                }
            };

            web_sys::console::log_1(&format!("🔧 toString called on XML {}", xml_id).into());

            // Check if it's a document
            let root_id = if let Some(doc) = player.xml_documents.get(&xml_id) {
                doc.root_element
            } else {
                // If it's a node, start from that node
                Some(xml_id)
            };

            if let Some(root_id) = root_id {
                let xml_string = Self::serialize_node(player, root_id);
                Ok(player.alloc_datum(Datum::String(xml_string)))
            } else {
                Ok(player.alloc_datum(Datum::String("".to_string())))
            }
        })
    }

    fn serialize_node(player: &DirPlayer, node_id: u32) -> String {
        if let Some(node) = player.xml_nodes.get(&node_id) {
            match node.node_type {
                XmlNodeType::Element => {
                    let mut xml = format!("<{}", node.name);

                    // Add attributes
                    for (key, value) in &node.attributes {
                        xml.push_str(&format!(" {}=\"{}\"", key, Self::escape_xml(value)));
                    }

                    // Check if has children
                    if node.children.is_empty() {
                        xml.push_str(" />");
                    } else {
                        xml.push('>');

                        // Serialize children
                        for child_id in &node.children {
                            xml.push_str(&Self::serialize_node(player, *child_id));
                        }

                        xml.push_str(&format!("</{}>", node.name));
                    }

                    xml
                }
                XmlNodeType::Text => {
                    if let Some(value) = &node.value {
                        Self::escape_xml(value)
                    } else {
                        String::new()
                    }
                }
                XmlNodeType::CData => {
                    if let Some(value) = &node.value {
                        format!("<![CDATA[{}]]>", value)
                    } else {
                        String::new()
                    }
                }
                XmlNodeType::Comment => {
                    if let Some(value) = &node.value {
                        format!("<!--{}-->", value)
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    // Get XML property
    fn get_xml_property(
        player: &mut DirPlayer,
        xml_id: u32,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        match prop {
            "firstChild" => {
                // Check if it's a document
                if let Some(doc) = player.xml_documents.get(&xml_id) {
                    if let Some(root_id) = doc.root_element {
                        return Ok(player.alloc_datum(Datum::XmlRef(root_id)));
                    } else {
                        web_sys::console::log_1(
                            &format!("🔧 ⚠️ Document {} has no root_element (None)", xml_id).into(),
                        );
                    }

                    return Ok(player.alloc_datum(Datum::Void));
                }

                // Check if it's a node
                if let Some(node) = player.xml_nodes.get(&xml_id) {
                    let children = XmlHelper::get_node_children(player, xml_id);
                    if let Some(first_child) = children.first() {
                        return Ok(first_child.clone());
                    } else {
                        web_sys::console::log_1(
                            &format!("🔧 Node has no accessible children").into(),
                        );
                    }
                } else {
                    web_sys::console::log_1(
                        &format!("🔧 ⚠️ xml_id {} is neither a document nor a node!", xml_id)
                            .into(),
                    );
                    web_sys::console::log_1(
                        &format!(
                            "🔧 Documents in storage: {:?}",
                            player.xml_documents.keys().collect::<Vec<_>>()
                        )
                        .into(),
                    );
                    web_sys::console::log_1(
                        &format!(
                            "🔧 Nodes in storage: {:?}",
                            player.xml_nodes.keys().collect::<Vec<_>>()
                        )
                        .into(),
                    );
                }

                Ok(player.alloc_datum(Datum::Void))
            }
            "lastChild" => {
                if let Some(node) = player.xml_nodes.get(&xml_id) {
                    let children = XmlHelper::get_node_children(player, xml_id);
                    if let Some(last_child) = children.last() {
                        return Ok(last_child.clone());
                    }
                }
                Ok(player.alloc_datum(Datum::Void))
            }
            // "childNodes" => {
            //     // If this is a document, return a list containing just the root element
            //     if let Some(doc) = player.xml_documents.get(&xml_id) {
            //         if let Some(root_id) = doc.root_element {
            //             web_sys::console::log_1(&format!("🔧 Document {}.childNodes returning root element {}", xml_id, root_id).into());
            //             let root_ref = player.alloc_datum(Datum::XmlRef(root_id));
            //             return Ok(player.alloc_datum(Datum::List(
            //                 crate::director::lingo::datum::DatumType::XmlChildNodes,
            //                 vec![root_ref],
            //                 false
            //             )));
            //         } else {
            //             // Document has no root element
            //             return Ok(player.alloc_datum(Datum::List(
            //                 crate::director::lingo::datum::DatumType::XmlChildNodes,
            //                 vec![],
            //                 false
            //             )));
            //         }
            //     }

            //     // For regular nodes, get their children
            //     let children = XmlHelper::get_node_children(player, xml_id);
            //     Ok(player.alloc_datum(Datum::List(
            //         crate::director::lingo::datum::DatumType::XmlChildNodes,
            //         children,
            //         false
            //     )))
            // },
            "childNodes" => {
                // If this is a document, return a list containing just the root element
                if let Some(doc) = player.xml_documents.get(&xml_id) {
                    if let Some(root_id) = doc.root_element {
                        web_sys::console::log_1(
                            &format!(
                                "🔧 Document {}.childNodes returning root element {}",
                                xml_id, root_id
                            )
                            .into(),
                        );
                        let root_ref = player.alloc_datum(Datum::XmlRef(root_id));
                        return Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::XmlChildNodes,
                            vec![root_ref],
                            false,
                        )));
                    } else {
                        // Document has no root element
                        return Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::XmlChildNodes,
                            vec![],
                            false,
                        )));
                    }
                }

                // For regular nodes, get their children
                let children = XmlHelper::get_node_children(player, xml_id);
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::XmlChildNodes,
                    children,
                    false,
                )))
            }
            "nodeName" => {
                if let Some(node) = player.xml_nodes.get(&xml_id) {
                    Ok(player.alloc_datum(Datum::String(Self::clean_node_name(&node.name))))
                } else {
                    Ok(player.alloc_datum(Datum::String("#document".to_string())))
                }
            }
            "nodeValue" => {
                if let Some(node) = player.xml_nodes.get(&xml_id) {
                    if let Some(value) = &node.value {
                        Ok(player.alloc_datum(Datum::String(value.clone())))
                    } else {
                        Ok(player.alloc_datum(Datum::Void))
                    }
                } else {
                    Ok(player.alloc_datum(Datum::Void))
                }
            }
            "attributes" => {
                let attr_id = xml_id + 10000;
                Ok(player.alloc_datum(Datum::XmlRef(attr_id)))
            }
            "ignoreWhite" => {
                let ignore = player
                    .xml_documents
                    .get(&xml_id)
                    .map(|doc| doc.ignore_white)
                    .unwrap_or(false);
                Ok(player.alloc_datum(Datum::Int(if ignore { 1 } else { 0 })))
            }
            attr_name if xml_id > 10000 => {
                let node_id = xml_id - 10000;

                if let Some(node) = player.xml_nodes.get(&node_id) {
                    let value = node
                        .attributes
                        .get(&attr_name.to_lowercase())
                        .cloned()
                        .unwrap_or_default();
                    Ok(player.alloc_datum(Datum::String(value)))
                } else {
                    web_sys::console::log_1(&format!("🔧 ⚠️ Node {} not found", node_id).into());
                    Ok(player.alloc_datum(Datum::String("".to_string())))
                }
            }
            _ => Err(ScriptError::new(format!("Unknown XML property: {}", prop))),
        }
    }

    fn clean_node_name(name: &str) -> String {
        name.trim_matches('"').to_lowercase()
    }

    fn set_xml_property(
        player: &mut DirPlayer,
        xml_id: u32,
        prop: &str,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        match prop {
            "ignoreWhite" => {
                let ignore_value = player.get_datum(value).int_value()? != 0;
                if let Some(doc) = player.xml_documents.get_mut(&xml_id) {
                    doc.ignore_white = ignore_value;
                }
                Ok(())
            }
            attr_name if xml_id > 10000 => {
                // Setting an attribute on an element
                let node_id = xml_id - 10000;
                let value_str = player.get_datum(value).string_value()?;

                web_sys::console::log_1(
                    &format!(
                        "🔧 Set attribute '{}' = '{}' on node {}",
                        attr_name, value_str, node_id
                    )
                    .into(),
                );

                if let Some(node) = player.xml_nodes.get_mut(&node_id) {
                    node.attributes.insert(attr_name.to_string(), value_str);
                    Ok(())
                } else {
                    Err(ScriptError::new(format!("Node {} not found", node_id)))
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set XML property: {}",
                prop
            ))),
        }
    }

    fn get_datum_type_name(datum: &Datum) -> &'static str {
        match datum {
            Datum::Void => "Void",
            Datum::Int(_) => "Int",
            Datum::Float(_) => "Float",
            Datum::String(_) => "String",
            Datum::Symbol(_) => "Symbol",
            Datum::List(_, _, _) => "List",
            Datum::PropList(_, _) => "PropList",
            Datum::XmlRef(_) => "XmlRef",
            Datum::ScriptRef(_) => "ScriptRef",
            Datum::CastLib(_) => "CastLib",
            Datum::TimeoutRef(_) => "TimeoutRef",
            _ => "Unknown",
        }
    }
}
