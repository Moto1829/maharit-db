//! Full-text search index implementation with BM25 scoring.
//!
//! This module provides a full-text search capability for graph nodes, allowing
//! efficient text search across node properties with relevance ranking.

use crate::{NodeId, PropertyValue};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors that can occur during full-text search operations.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FulltextError {
    #[error("index not found: {0}")]
    IndexNotFound(String),
    #[error("index already exists: {0}")]
    IndexAlreadyExists(String),
}

/// A search result with relevance score.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub node_id: NodeId,
    pub score: f64,
}

impl SearchResult {
    fn new(node_id: NodeId, score: f64) -> Self {
        Self { node_id, score }
    }
}

/// Simple tokenizer that splits text on non-alphanumeric characters and lowercases.
struct Tokenizer;

impl Tokenizer {
    /// Tokenize text into lowercase tokens, filtering out empty strings.
    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

/// Document identifier combining node ID and property name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DocumentId {
    node_id: NodeId,
    property: String,
}

impl DocumentId {
    fn new(node_id: NodeId, property: &str) -> Self {
        Self {
            node_id,
            property: property.to_string(),
        }
    }
}

/// Position information for a token in a document.
#[derive(Debug, Clone)]
struct TokenPosition {
    doc_id: DocumentId,
    positions: Vec<usize>,
}

/// Inverted index mapping tokens to documents and positions.
#[derive(Debug, Clone)]
struct InvertedIndex {
    /// Maps tokens to their occurrences in documents
    index: HashMap<String, Vec<TokenPosition>>,
    /// Document lengths (number of tokens) for BM25
    doc_lengths: HashMap<DocumentId, usize>,
    /// Total number of documents
    total_docs: usize,
    /// Sum of all document lengths for average calculation
    total_length: usize,
}

impl InvertedIndex {
    fn new() -> Self {
        Self {
            index: HashMap::new(),
            doc_lengths: HashMap::new(),
            total_docs: 0,
            total_length: 0,
        }
    }

    /// Add a document to the inverted index.
    fn add_document(&mut self, doc_id: DocumentId, text: &str) {
        let tokens = Tokenizer::tokenize(text);
        let doc_length = tokens.len();

        // Update document length tracking
        if !self.doc_lengths.contains_key(&doc_id) {
            self.total_docs += 1;
        } else {
            // Remove old length from total
            if let Some(&old_length) = self.doc_lengths.get(&doc_id) {
                self.total_length -= old_length;
            }
        }

        self.doc_lengths.insert(doc_id.clone(), doc_length);
        self.total_length += doc_length;

        // Build position map for this document
        let mut token_positions: HashMap<String, Vec<usize>> = HashMap::new();
        for (pos, token) in tokens.iter().enumerate() {
            token_positions
                .entry(token.clone())
                .or_insert_with(Vec::new)
                .push(pos);
        }

        // Update inverted index
        for (token, positions) in token_positions {
            let entry = self.index.entry(token).or_insert_with(Vec::new);

            // Remove existing entry for this document if present
            entry.retain(|tp| tp.doc_id != doc_id);

            // Add new entry
            entry.push(TokenPosition {
                doc_id: doc_id.clone(),
                positions,
            });
        }
    }

    /// Remove a document from the inverted index.
    fn remove_document(&mut self, doc_id: &DocumentId) {
        // Remove from all token postings lists
        for postings in self.index.values_mut() {
            postings.retain(|tp| tp.doc_id != *doc_id);
        }

        // Update document statistics
        if let Some(length) = self.doc_lengths.remove(doc_id) {
            self.total_docs = self.total_docs.saturating_sub(1);
            self.total_length = self.total_length.saturating_sub(length);
        }

        // Clean up empty token entries
        self.index.retain(|_, postings| !postings.is_empty());
    }

    /// Get average document length.
    fn avg_doc_length(&self) -> f64 {
        if self.total_docs == 0 {
            0.0
        } else {
            self.total_length as f64 / self.total_docs as f64
        }
    }

    /// Get document frequency for a token.
    fn doc_frequency(&self, token: &str) -> usize {
        self.index.get(token).map(|v| v.len()).unwrap_or(0)
    }

    /// Get term frequency for a token in a specific document.
    fn term_frequency(&self, token: &str, doc_id: &DocumentId) -> usize {
        self.index
            .get(token)
            .and_then(|postings| {
                postings
                    .iter()
                    .find(|tp| tp.doc_id == *doc_id)
                    .map(|tp| tp.positions.len())
            })
            .unwrap_or(0)
    }

    /// Get all documents containing a token.
    fn documents_with_token(&self, token: &str) -> HashSet<DocumentId> {
        self.index
            .get(token)
            .map(|postings| postings.iter().map(|tp| tp.doc_id.clone()).collect())
            .unwrap_or_else(HashSet::new)
    }
}

/// A full-text search index for a specific label and set of properties.
#[derive(Debug, Clone)]
pub struct FulltextIndex {
    name: String,
    label: String,
    properties: Vec<String>,
    inverted_index: InvertedIndex,
    /// BM25 parameter k1 (term frequency saturation)
    k1: f64,
    /// BM25 parameter b (length normalization)
    b: f64,
}

impl FulltextIndex {
    /// Create a new fulltext index.
    pub fn new(name: &str, label: &str, properties: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            label: label.to_string(),
            properties,
            inverted_index: InvertedIndex::new(),
            k1: 1.2,
            b: 0.75,
        }
    }

    /// Get the index name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the label this index is for.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Get the properties this index covers.
    pub fn properties(&self) -> &[String] {
        &self.properties
    }

    /// Add a document to the index.
    pub fn add_document(&mut self, node_id: NodeId, property: &str, text: &str) {
        let doc_id = DocumentId::new(node_id, property);
        self.inverted_index.add_document(doc_id, text);
    }

    /// Remove a document from the index.
    pub fn remove_document(&mut self, node_id: NodeId, property: &str) {
        let doc_id = DocumentId::new(node_id, property);
        self.inverted_index.remove_document(&doc_id);
    }

    /// Calculate BM25 score for a document given query tokens.
    fn calculate_bm25(&self, doc_id: &DocumentId, query_tokens: &[String]) -> f64 {
        let doc_length = self
            .inverted_index
            .doc_lengths
            .get(doc_id)
            .copied()
            .unwrap_or(0) as f64;
        let avg_length = self.inverted_index.avg_doc_length();
        let total_docs = self.inverted_index.total_docs as f64;

        let mut score = 0.0;

        for token in query_tokens {
            let tf = self.inverted_index.term_frequency(token, doc_id) as f64;
            let df = self.inverted_index.doc_frequency(token) as f64;

            if tf > 0.0 {
                // IDF calculation using BM25+ variant: always positive
                let idf = (1.0 + (total_docs - df + 0.5) / (df + 0.5)).ln();

                // BM25 formula
                let numerator = tf * (self.k1 + 1.0);
                let denominator = tf + self.k1 * (1.0 - self.b + self.b * (doc_length / avg_length));

                score += idf * (numerator / denominator);
            }
        }

        score
    }

    /// Search the index and return ranked results.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let query_tokens = Tokenizer::tokenize(query);

        if query_tokens.is_empty() {
            return Vec::new();
        }

        // Find all documents containing at least one query token
        let mut candidate_docs = HashSet::new();
        for token in &query_tokens {
            candidate_docs.extend(self.inverted_index.documents_with_token(token));
        }

        // Score each candidate document
        let results: Vec<SearchResult> = candidate_docs
            .iter()
            .map(|doc_id| {
                let score = self.calculate_bm25(doc_id, &query_tokens);
                SearchResult::new(doc_id.node_id, score)
            })
            .collect();

        // Aggregate scores by node_id (a node may have multiple properties indexed)
        let mut node_scores: HashMap<NodeId, f64> = HashMap::new();
        for result in results {
            *node_scores.entry(result.node_id).or_insert(0.0) += result.score;
        }

        // Convert back to results and sort by score (descending)
        let mut final_results: Vec<SearchResult> = node_scores
            .into_iter()
            .map(|(node_id, score)| SearchResult::new(node_id, score))
            .collect();

        final_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        final_results
    }

    /// Check if documents contain all query terms (AND semantics).
    pub fn contains(&self, query: &str) -> HashSet<NodeId> {
        let query_tokens = Tokenizer::tokenize(query);

        if query_tokens.is_empty() {
            return HashSet::new();
        }

        // Start with documents containing the first token
        let mut result_docs = self.inverted_index.documents_with_token(&query_tokens[0]);

        // Intersect with documents containing each subsequent token
        for token in query_tokens.iter().skip(1) {
            let docs_with_token = self.inverted_index.documents_with_token(token);
            result_docs.retain(|doc| docs_with_token.contains(doc));
        }

        // Extract unique node IDs
        result_docs.iter().map(|doc| doc.node_id).collect()
    }
}

/// Manager for multiple fulltext indexes.
#[derive(Debug, Clone)]
pub struct FulltextManager {
    indexes: HashMap<String, FulltextIndex>,
}

impl FulltextManager {
    /// Create a new fulltext manager.
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
        }
    }

    /// Create a new fulltext index.
    pub fn create_index(
        &mut self,
        name: &str,
        label: &str,
        properties: Vec<String>,
    ) -> Result<(), FulltextError> {
        if self.indexes.contains_key(name) {
            return Err(FulltextError::IndexAlreadyExists(name.to_string()));
        }

        let index = FulltextIndex::new(name, label, properties);
        self.indexes.insert(name.to_string(), index);
        Ok(())
    }

    /// Drop a fulltext index.
    pub fn drop_index(&mut self, name: &str) -> Result<(), FulltextError> {
        self.indexes
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| FulltextError::IndexNotFound(name.to_string()))
    }

    /// Get an immutable reference to an index.
    pub fn get_index(&self, name: &str) -> Option<&FulltextIndex> {
        self.indexes.get(name)
    }

    /// Get a mutable reference to an index.
    pub fn get_index_mut(&mut self, name: &str) -> Option<&mut FulltextIndex> {
        self.indexes.get_mut(name)
    }

    /// Index a node's properties into matching indexes.
    pub fn index_node(
        &mut self,
        node_id: NodeId,
        label: &str,
        properties: &HashMap<String, PropertyValue>,
    ) {
        // Find all indexes matching this label
        let matching_indexes: Vec<String> = self
            .indexes
            .values()
            .filter(|idx| idx.label() == label)
            .map(|idx| idx.name().to_string())
            .collect();

        for index_name in matching_indexes {
            if let Some(index) = self.indexes.get_mut(&index_name) {
                // Clone properties list to avoid borrow conflict
                let index_props: Vec<String> = index.properties().to_vec();
                for property_name in &index_props {
                    if let Some(value) = properties.get(property_name.as_str()) {
                        // Only index string properties
                        if let PropertyValue::String(text) = value {
                            index.add_document(node_id, property_name, text);
                        }
                    }
                }
            }
        }
    }

    /// Remove a node from all indexes.
    pub fn remove_node(&mut self, node_id: NodeId) {
        for index in self.indexes.values_mut() {
            let props: Vec<String> = index.properties().to_vec();
            for property in &props {
                index.remove_document(node_id, property);
            }
        }
    }

    /// List all indexes.
    pub fn list_indexes(&self) -> Vec<&FulltextIndex> {
        self.indexes.values().collect()
    }

    /// Search an index by name.
    pub fn search(&self, index_name: &str, query: &str) -> Result<Vec<SearchResult>, FulltextError> {
        self.indexes
            .get(index_name)
            .map(|idx| idx.search(query))
            .ok_or_else(|| FulltextError::IndexNotFound(index_name.to_string()))
    }

    /// Check containment in an index by name.
    pub fn contains(
        &self,
        index_name: &str,
        query: &str,
    ) -> Result<HashSet<NodeId>, FulltextError> {
        self.indexes
            .get(index_name)
            .map(|idx| idx.contains(query))
            .ok_or_else(|| FulltextError::IndexNotFound(index_name.to_string()))
    }
}

impl Default for FulltextManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenizer_basic() {
        let tokens = Tokenizer::tokenize("Hello World");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenizer_punctuation() {
        let tokens = Tokenizer::tokenize("Hello, World! How are you?");
        assert_eq!(tokens, vec!["hello", "world", "how", "are", "you"]);
    }

    #[test]
    fn test_tokenizer_case_insensitive() {
        let tokens1 = Tokenizer::tokenize("HELLO");
        let tokens2 = Tokenizer::tokenize("hello");
        assert_eq!(tokens1, tokens2);
    }

    #[test]
    fn test_add_and_remove_document() {
        let mut index = FulltextIndex::new("test", "Person", vec!["name".to_string()]);

        index.add_document(1, "name", "Alice Johnson");
        assert_eq!(index.inverted_index.total_docs, 1);

        index.remove_document(1, "name");
        assert_eq!(index.inverted_index.total_docs, 0);
    }

    #[test]
    fn test_simple_search() {
        let mut index = FulltextIndex::new("test", "Person", vec!["name".to_string()]);

        index.add_document(1, "name", "Alice Johnson");
        index.add_document(2, "name", "Bob Smith");
        index.add_document(3, "name", "Alice Cooper");

        let results = index.search("Alice");
        assert_eq!(results.len(), 2);

        let node_ids: HashSet<NodeId> = results.iter().map(|r| r.node_id).collect();
        assert!(node_ids.contains(&1));
        assert!(node_ids.contains(&3));
    }

    #[test]
    fn test_bm25_scoring() {
        let mut index = FulltextIndex::new("test", "Document", vec!["content".to_string()]);

        // Document with query term appearing once in a short doc
        index.add_document(1, "content", "Rust programming");

        // Document with query term appearing multiple times
        index.add_document(2, "content", "Rust is great Rust is awesome Rust");

        // A document with no match
        index.add_document(3, "content", "Python programming language");

        let results = index.search("Rust");
        assert_eq!(results.len(), 2);

        // Both matching docs should have positive scores
        assert!(results[0].score > 0.0);
        assert!(results[1].score > 0.0);

        // Doc 3 should not appear
        let node_ids: HashSet<NodeId> = results.iter().map(|r| r.node_id).collect();
        assert!(!node_ids.contains(&3));
    }

    #[test]
    fn test_contains_and_semantics() {
        let mut index = FulltextIndex::new("test", "Article", vec!["text".to_string()]);

        index.add_document(1, "text", "Rust programming language");
        index.add_document(2, "text", "Python programming");
        index.add_document(3, "text", "Rust language");

        // All terms must match
        let results = index.contains("Rust programming");
        assert_eq!(results.len(), 1);
        assert!(results.contains(&1));

        // Single term
        let results = index.contains("Rust");
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&3));
    }

    #[test]
    fn test_fulltext_manager_create_drop() {
        let mut manager = FulltextManager::new();

        let result = manager.create_index("idx1", "Person", vec!["name".to_string()]);
        assert!(result.is_ok());

        // Duplicate creation should fail
        let result = manager.create_index("idx1", "Person", vec!["name".to_string()]);
        assert!(matches!(result, Err(FulltextError::IndexAlreadyExists(_))));

        // Drop should succeed
        let result = manager.drop_index("idx1");
        assert!(result.is_ok());

        // Drop non-existent should fail
        let result = manager.drop_index("idx1");
        assert!(matches!(result, Err(FulltextError::IndexNotFound(_))));
    }

    #[test]
    fn test_fulltext_manager_list() {
        let mut manager = FulltextManager::new();

        manager.create_index("idx1", "Person", vec!["name".to_string()]).unwrap();
        manager.create_index("idx2", "Article", vec!["title".to_string()]).unwrap();

        let indexes = manager.list_indexes();
        assert_eq!(indexes.len(), 2);
    }

    #[test]
    fn test_index_node_integration() {
        let mut manager = FulltextManager::new();
        manager.create_index("person_idx", "Person", vec!["name".to_string(), "bio".to_string()]).unwrap();

        let mut properties = HashMap::new();
        properties.insert("name".to_string(), PropertyValue::String("Alice".to_string()));
        properties.insert("bio".to_string(), PropertyValue::String("Software engineer".to_string()));
        properties.insert("age".to_string(), PropertyValue::Int(30));

        manager.index_node(1, "Person", &properties);

        let results = manager.search("person_idx", "Alice").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);

        let results = manager.search("person_idx", "engineer").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);
    }

    #[test]
    fn test_search_nonexistent_index() {
        let manager = FulltextManager::new();
        let result = manager.search("nonexistent", "query");
        assert!(matches!(result, Err(FulltextError::IndexNotFound(_))));
    }

    #[test]
    fn test_empty_query() {
        let mut index = FulltextIndex::new("test", "Person", vec!["name".to_string()]);
        index.add_document(1, "name", "Alice");

        let results = index.search("");
        assert!(results.is_empty());

        let results = index.contains("");
        assert!(results.is_empty());
    }

    #[test]
    fn test_case_insensitive_search() {
        let mut index = FulltextIndex::new("test", "Person", vec!["name".to_string()]);
        index.add_document(1, "name", "Alice Johnson");

        let results1 = index.search("ALICE");
        let results2 = index.search("alice");
        let results3 = index.search("Alice");

        assert_eq!(results1.len(), 1);
        assert_eq!(results2.len(), 1);
        assert_eq!(results3.len(), 1);

        assert_eq!(results1[0].node_id, 1);
        assert_eq!(results2[0].node_id, 1);
        assert_eq!(results3[0].node_id, 1);
    }

    #[test]
    fn test_remove_node_from_manager() {
        let mut manager = FulltextManager::new();
        manager.create_index("idx1", "Person", vec!["name".to_string()]).unwrap();

        let mut properties = HashMap::new();
        properties.insert("name".to_string(), PropertyValue::String("Alice".to_string()));

        manager.index_node(1, "Person", &properties);

        let results = manager.search("idx1", "Alice").unwrap();
        assert_eq!(results.len(), 1);

        manager.remove_node(1);

        let results = manager.search("idx1", "Alice").unwrap();
        assert!(results.is_empty());
    }
}
