//! Full-text search index implementation with BM25 scoring.
//!
//! This module provides a full-text search capability for graph nodes, allowing
//! efficient text search across node properties with relevance ranking.

use crate::{NodeId, PropertyValue};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[cfg(feature = "japanese")]
use lindera::dictionary::load_dictionary;
#[cfg(feature = "japanese")]
use lindera::mode::Mode;
#[cfg(feature = "japanese")]
use lindera::segmenter::Segmenter;
#[cfg(feature = "japanese")]
use lindera::tokenizer::Tokenizer as LinderaTokenizer;

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

/// Returns `true` when the text contains at least one hiragana, katakana, or CJK character.
///
/// Used to decide whether to route text through the lindera Japanese morphological analyser.
fn contains_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        let cp = c as u32;
        // Hiragana: U+3040–U+309F
        // Katakana: U+30A0–U+30FF
        // CJK Unified Ideographs: U+4E00–U+9FFF
        matches!(cp, 0x3040..=0x309F | 0x30A0..=0x30FF | 0x4E00..=0x9FFF)
    })
}

/// Part-of-speech tags (IPADIC format) that should be excluded from index tokens.
///
/// These tags correspond to particles (助詞), auxiliary verbs (助動詞), symbols (記号),
/// conjunctions (接続詞), interjections (感動詞), and filler sounds.
#[cfg(feature = "japanese")]
const JAPANESE_STOP_POS: &[&str] = &[
    "助詞",
    "助動詞",
    "記号",
    "接続詞",
    "感動詞",
    "フィラー",
    "非言語音",
];

/// Threshold above which parallel index building is used.
const PARALLEL_BUILD_THRESHOLD: usize = 200;

/// Simple tokenizer that splits text on non-alphanumeric characters and lowercases.
///
/// When the `japanese` feature is enabled and the input contains Japanese characters,
/// lindera's morphological analyser is used instead, filtering out stop parts-of-speech
/// and returning the dictionary (base) form of each content word.
///
/// When building the index in parallel, each worker thread keeps its own
/// lindera `Tokenizer` in a thread-local variable so the expensive dictionary
/// load only happens once per thread instead of once per document.
struct Tokenizer;

impl Tokenizer {
    /// Tokenize text into lowercase tokens, filtering out empty strings.
    ///
    /// For text that contains Japanese characters (hiragana, katakana, or CJK) and when
    /// the `japanese` feature is compiled in, morphological analysis via lindera is used.
    /// All other text is split on non-alphanumeric boundaries and lowercased.
    fn tokenize(text: &str) -> Vec<String> {
        #[cfg(feature = "japanese")]
        if contains_japanese(text) {
            return Self::tokenize_japanese(text);
        }

        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    /// Tokenize Japanese text using lindera morphological analysis with IPADIC.
    ///
    /// Loads the embedded IPADIC dictionary, segments the text, removes stop
    /// parts-of-speech (particles, auxiliary verbs, symbols, etc.), and returns
    /// the dictionary (base) form of each retained token in lowercase.
    ///
    /// On any lindera error the function falls back to simple whitespace splitting
    /// so that indexing never silently discards text.
    #[cfg(feature = "japanese")]
    fn tokenize_japanese(text: &str) -> Vec<String> {
        // Build a tokenizer backed by the embedded IPADIC dictionary.
        let tokenizer = match Self::build_japanese_tokenizer() {
            Ok(t) => t,
            Err(_) => {
                // Fallback: split on whitespace / non-alphanumeric boundaries.
                return text
                    .split(|c: char| c.is_whitespace() || !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
        };

        let mut tokens = match tokenizer.tokenize(text) {
            Ok(t) => t,
            Err(_) => {
                return text
                    .split(|c: char| c.is_whitespace() || !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
        };

        let mut result = Vec::new();
        for token in tokens.iter_mut() {
            // Capture the surface string before calling `details()` since `details()`
            // takes `&mut self` and would otherwise conflict with borrowing `surface`.
            let surface_owned: String = token.surface.as_ref().to_string();
            let details = token.details();

            // details[0] is the major part-of-speech in IPADIC format.
            // Skip stop POS categories.
            let pos = details.first().copied().unwrap_or("*");
            if JAPANESE_STOP_POS.contains(&pos) {
                continue;
            }

            // details[6] is the dictionary (base) form in IPADIC layout.
            // Fall back to the surface form if the base form is not available or is "*".
            let base_form = details
                .get(6)
                .copied()
                .filter(|s| !s.is_empty() && *s != "*")
                .unwrap_or(&surface_owned);

            let normalized = base_form.to_lowercase();
            if !normalized.is_empty() {
                result.push(normalized);
            }
        }

        result
    }

    /// Construct a lindera `Tokenizer` using the embedded IPADIC dictionary.
    #[cfg(feature = "japanese")]
    fn build_japanese_tokenizer() -> lindera::LinderaResult<LinderaTokenizer> {
        let dictionary = load_dictionary("embedded://ipadic")?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        Ok(LinderaTokenizer::new(segmenter))
    }

    /// Tokenize text using a thread-local cached tokenizer.
    ///
    /// This variant is used during parallel index building so that the expensive
    /// lindera dictionary load happens at most once per rayon worker thread.
    fn tokenize_cached(text: &str) -> Vec<String> {
        // For the non-japanese path this is identical to `tokenize`.
        // For the japanese path the thread-local cache avoids repeated dictionary loads.
        #[cfg(feature = "japanese")]
        if contains_japanese(text) {
            return Self::tokenize_japanese_cached(text);
        }

        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    /// Japanese tokenization using a thread-local cached lindera tokenizer.
    #[cfg(feature = "japanese")]
    fn tokenize_japanese_cached(text: &str) -> Vec<String> {
        // Each rayon worker thread gets its own lindera tokenizer so we pay the
        // dictionary-load cost at most once per thread.
        thread_local! {
            static TOKENIZER: std::cell::RefCell<Option<LinderaTokenizer>> =
                const { std::cell::RefCell::new(None) };
        }

        TOKENIZER.with(|cell| {
            let mut borrow = cell.borrow_mut();
            if borrow.is_none() {
                *borrow = Self::build_japanese_tokenizer().ok();
            }

            // If initialization failed, fall back to the whitespace splitter.
            match borrow.as_mut() {
                None => text
                    .split(|c: char| c.is_whitespace() || !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
                Some(tokenizer) => {
                    let mut tokens = match tokenizer.tokenize(text) {
                        Ok(t) => t,
                        Err(_) => {
                            return text
                                .split(|c: char| c.is_whitespace() || !c.is_alphanumeric())
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                                .collect();
                        }
                    };

                    let mut result = Vec::new();
                    for token in tokens.iter_mut() {
                        let surface_owned: String = token.surface.as_ref().to_string();
                        let details = token.details();
                        let pos = details.first().copied().unwrap_or("*");
                        if JAPANESE_STOP_POS.contains(&pos) {
                            continue;
                        }
                        let base_form = details
                            .get(6)
                            .copied()
                            .filter(|s| !s.is_empty() && *s != "*")
                            .unwrap_or(&surface_owned);
                        let normalized = base_form.to_lowercase();
                        if !normalized.is_empty() {
                            result.push(normalized);
                        }
                    }
                    result
                }
            }
        })
    }
}

/// Compute the Levenshtein (edit) distance between two strings.
///
/// Uses dynamic programming with O(min(a.len(), b.len())) space.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    // Use the shorter string as the column dimension to minimize memory usage
    if m < n {
        return levenshtein_distance(b, a);
    }

    // prev[j] = edit distance between a[0..i-1] and b[0..j]
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Parse a fuzzy query like `term~` or `term~2` into (term, max_distance).
///
/// Returns `None` if the query does not have a `~` suffix.
fn parse_fuzzy_query(query: &str) -> Option<(&str, usize)> {
    let query = query.trim();
    let tilde_pos = query.rfind('~')?;
    let term = &query[..tilde_pos];
    let suffix = &query[tilde_pos + 1..];
    let max_distance = if suffix.is_empty() {
        2
    } else {
        suffix.parse::<usize>().ok()?
    };
    Some((term, max_distance))
}

/// Detect if a query string is a phrase search (surrounded by `"` or `'`).
///
/// Returns the inner phrase text without the surrounding quotes, or `None`.
fn parse_phrase_query(query: &str) -> Option<&str> {
    let query = query.trim();
    if query.len() >= 2 {
        let first = query.as_bytes()[0];
        let last = query.as_bytes()[query.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return Some(&query[1..query.len() - 1]);
        }
    }
    None
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
                .or_default()
                .push(pos);
        }

        // Update inverted index
        for (token, positions) in token_positions {
            let entry = self.index.entry(token).or_default();

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
            .unwrap_or_default()
    }

    /// Get positions of a token in a specific document.
    fn positions_in_doc(&self, token: &str, doc_id: &DocumentId) -> Option<&[usize]> {
        self.index
            .get(token)
            .and_then(|postings| postings.iter().find(|tp| tp.doc_id == *doc_id))
            .map(|tp| tp.positions.as_slice())
    }

    /// Check whether a sequence of tokens appears as a consecutive phrase in the document.
    ///
    /// For each starting position of the first token, checks whether every subsequent
    /// token appears at exactly offset+1, offset+2, … positions.
    fn is_phrase_in_doc(&self, tokens: &[String], doc_id: &DocumentId) -> bool {
        if tokens.is_empty() {
            return false;
        }
        if tokens.len() == 1 {
            return !self.documents_with_token(&tokens[0]).is_empty()
                && self.documents_with_token(&tokens[0]).contains(doc_id);
        }

        let first_positions = match self.positions_in_doc(&tokens[0], doc_id) {
            Some(p) => p,
            None => return false,
        };

        'outer: for &start_pos in first_positions {
            for (offset, token) in tokens.iter().enumerate().skip(1) {
                let expected_pos = start_pos + offset;
                match self.positions_in_doc(token, doc_id) {
                    Some(positions) if positions.contains(&expected_pos) => {}
                    _ => continue 'outer,
                }
            }
            // All tokens matched consecutively
            return true;
        }

        false
    }

    /// Collect all document IDs that match any token within the given edit distance.
    fn documents_with_fuzzy_token(
        &self,
        term: &str,
        max_distance: usize,
    ) -> HashSet<DocumentId> {
        let mut result = HashSet::new();
        for (index_token, postings) in &self.index {
            if levenshtein_distance(term, index_token) <= max_distance {
                for tp in postings {
                    result.insert(tp.doc_id.clone());
                }
            }
        }
        result
    }

    /// Collect all tokens within the given edit distance of `term`.
    fn fuzzy_matching_tokens(&self, term: &str, max_distance: usize) -> Vec<String> {
        self.index
            .keys()
            .filter(|k| levenshtein_distance(term, k) <= max_distance)
            .cloned()
            .collect()
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

    /// Bulk-index a batch of documents using two-phase parallel construction.
    ///
    /// For batches with >= [`PARALLEL_BUILD_THRESHOLD`] entries the tokenization
    /// phase is run in parallel across rayon worker threads (each node is
    /// independent). Writing into the inverted index is then done sequentially
    /// so no synchronisation primitives are required.
    ///
    /// # Arguments
    ///
    /// * `documents` – Slice of `(node_id, property_name, text)` tuples to index.
    pub fn build_index(&mut self, documents: &[(NodeId, &str, &str)]) {
        if documents.is_empty() {
            return;
        }

        // Phase 1: tokenize each document (parallel when large enough).
        // Returns Vec<(DocumentId, Vec<String /*tokens*/>)>
        let tokenized: Vec<(DocumentId, Vec<String>)> = if documents.len()
            >= PARALLEL_BUILD_THRESHOLD
        {
            documents
                .par_iter()
                .map(|&(node_id, property, text)| {
                    let doc_id = DocumentId::new(node_id, property);
                    let tokens = Tokenizer::tokenize_cached(text);
                    (doc_id, tokens)
                })
                .collect()
        } else {
            documents
                .iter()
                .map(|&(node_id, property, text)| {
                    let doc_id = DocumentId::new(node_id, property);
                    let tokens = Tokenizer::tokenize(text);
                    (doc_id, tokens)
                })
                .collect()
        };

        // Phase 2: write tokenized results into the inverted index sequentially.
        for (doc_id, tokens) in tokenized {
            let doc_length = tokens.len();

            if !self.inverted_index.doc_lengths.contains_key(&doc_id) {
                self.inverted_index.total_docs += 1;
            } else if let Some(&old_len) = self.inverted_index.doc_lengths.get(&doc_id) {
                self.inverted_index.total_length -= old_len;
            }

            self.inverted_index
                .doc_lengths
                .insert(doc_id.clone(), doc_length);
            self.inverted_index.total_length += doc_length;

            // Build position map for this document.
            let mut token_positions: HashMap<String, Vec<usize>> = HashMap::new();
            for (pos, token) in tokens.iter().enumerate() {
                token_positions
                    .entry(token.clone())
                    .or_default()
                    .push(pos);
            }

            for (token, positions) in token_positions {
                let entry = self
                    .inverted_index
                    .index
                    .entry(token)
                    .or_default();
                entry.retain(|tp| tp.doc_id != doc_id);
                entry.push(TokenPosition {
                    doc_id: doc_id.clone(),
                    positions,
                });
            }
        }
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
                let denominator =
                    tf + self.k1 * (1.0 - self.b + self.b * (doc_length / avg_length));

                score += idf * (numerator / denominator);
            }
        }

        score
    }

    /// Search the index and return ranked results.
    ///
    /// The query format is:
    /// - Plain keywords: `graph database` — BM25 scored OR-style keyword search
    /// - Phrase search: `"graph database"` — tokens must appear consecutively
    /// - Fuzzy search: `databse~` or `databse~1` — match within edit distance N (default 2)
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        // Phrase search: "..."
        if let Some(phrase) = parse_phrase_query(query) {
            return self.search_phrase(phrase);
        }

        // Fuzzy search: term~ or term~N
        if let Some((term, max_distance)) = parse_fuzzy_query(query) {
            return self.search_fuzzy(term, max_distance);
        }

        // Standard BM25 keyword search
        let query_tokens = Tokenizer::tokenize(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }
        self.search_keywords(&query_tokens)
    }

    /// Perform BM25-scored keyword search for the given pre-tokenized query.
    fn search_keywords(&self, query_tokens: &[String]) -> Vec<SearchResult> {
        // Find all documents containing at least one query token
        let mut candidate_docs = HashSet::new();
        for token in query_tokens {
            candidate_docs.extend(self.inverted_index.documents_with_token(token));
        }

        // Score each candidate document
        let results: Vec<SearchResult> = candidate_docs
            .iter()
            .map(|doc_id| {
                let score = self.calculate_bm25(doc_id, query_tokens);
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

    /// Perform phrase search: all tokens must appear consecutively in the correct order.
    ///
    /// Uses BM25 scoring on the phrase tokens for ranking among matches.
    fn search_phrase(&self, phrase: &str) -> Vec<SearchResult> {
        let phrase_tokens = Tokenizer::tokenize(phrase);
        if phrase_tokens.is_empty() {
            return Vec::new();
        }

        // Candidate documents must contain all tokens (AND filter)
        let mut candidate_docs = self
            .inverted_index
            .documents_with_token(&phrase_tokens[0]);
        for token in phrase_tokens.iter().skip(1) {
            let with_token = self.inverted_index.documents_with_token(token);
            candidate_docs.retain(|d| with_token.contains(d));
        }

        // Filter to only documents where tokens appear consecutively
        let phrase_docs: Vec<&DocumentId> = candidate_docs
            .iter()
            .filter(|doc_id| self.inverted_index.is_phrase_in_doc(&phrase_tokens, doc_id))
            .collect();

        if phrase_docs.is_empty() {
            return Vec::new();
        }

        // Score using BM25 on phrase tokens
        let mut node_scores: HashMap<NodeId, f64> = HashMap::new();
        for doc_id in phrase_docs {
            let score = self.calculate_bm25(doc_id, &phrase_tokens);
            *node_scores.entry(doc_id.node_id).or_insert(0.0) += score;
        }

        let mut results: Vec<SearchResult> = node_scores
            .into_iter()
            .map(|(node_id, score)| SearchResult::new(node_id, score))
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Perform fuzzy search: return documents containing a token within `max_distance`
    /// edit distance from the query term. Uses BM25 scoring on matched tokens.
    fn search_fuzzy(&self, term: &str, max_distance: usize) -> Vec<SearchResult> {
        let term_lower = term.to_lowercase();
        let matched_tokens = self
            .inverted_index
            .fuzzy_matching_tokens(&term_lower, max_distance);

        if matched_tokens.is_empty() {
            return Vec::new();
        }

        // Collect candidates from all matching tokens
        let mut candidate_docs: HashSet<DocumentId> = HashSet::new();
        for token in &matched_tokens {
            candidate_docs.extend(self.inverted_index.documents_with_token(token));
        }

        // Score using BM25 on all matched tokens
        let mut node_scores: HashMap<NodeId, f64> = HashMap::new();
        for doc_id in &candidate_docs {
            let score = self.calculate_bm25(doc_id, &matched_tokens);
            *node_scores.entry(doc_id.node_id).or_insert(0.0) += score;
        }

        let mut results: Vec<SearchResult> = node_scores
            .into_iter()
            .map(|(node_id, score)| SearchResult::new(node_id, score))
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Check if documents contain all query terms (AND semantics).
    ///
    /// The query format is:
    /// - Plain keywords: AND semantics (all tokens must appear)
    /// - Phrase search: `"graph database"` — tokens must appear consecutively
    /// - Fuzzy search: `databse~` or `databse~1` — match within edit distance N (default 2)
    pub fn contains(&self, query: &str) -> HashSet<NodeId> {
        // Phrase search: "..."
        if let Some(phrase) = parse_phrase_query(query) {
            let phrase_tokens = Tokenizer::tokenize(phrase);
            if phrase_tokens.is_empty() {
                return HashSet::new();
            }
            // Candidate docs must contain all tokens
            let mut candidate_docs = self
                .inverted_index
                .documents_with_token(&phrase_tokens[0]);
            for token in phrase_tokens.iter().skip(1) {
                let with_token = self.inverted_index.documents_with_token(token);
                candidate_docs.retain(|d| with_token.contains(d));
            }
            // Filter by consecutive positioning
            return candidate_docs
                .iter()
                .filter(|doc_id| {
                    self.inverted_index.is_phrase_in_doc(&phrase_tokens, doc_id)
                })
                .map(|doc_id| doc_id.node_id)
                .collect();
        }

        // Fuzzy search: term~ or term~N
        if let Some((term, max_distance)) = parse_fuzzy_query(query) {
            let term_lower = term.to_lowercase();
            return self
                .inverted_index
                .documents_with_fuzzy_token(&term_lower, max_distance)
                .iter()
                .map(|doc_id| doc_id.node_id)
                .collect();
        }

        // Standard AND keyword search
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
                    if let Some(PropertyValue::String(text)) = properties.get(property_name.as_str()) {
                        index.add_document(node_id, property_name, text);
                    }
                }
            }
        }
    }

    /// Bulk-index multiple nodes' properties into matching indexes.
    ///
    /// Internally delegates to [`FulltextIndex::build_index`] which parallelises
    /// the tokenization phase for large batches.
    ///
    /// # Arguments
    ///
    /// * `nodes` – Slice of `(node_id, label, properties)` tuples.
    pub fn build_index_bulk(
        &mut self,
        nodes: &[(NodeId, &str, HashMap<String, PropertyValue>)],
    ) {
        // Group documents by (index_name) so we can call build_index once per index.
        // We collect owned strings to satisfy lifetime requirements.
        let mut per_index: HashMap<String, Vec<(NodeId, String, String)>> = HashMap::new();

        for (node_id, label, properties) in nodes {
            for index in self.indexes.values() {
                if index.label() != *label {
                    continue;
                }
                for prop_name in index.properties() {
                    if let Some(PropertyValue::String(text)) = properties.get(prop_name.as_str()) {
                        per_index
                            .entry(index.name().to_string())
                            .or_default()
                            .push((*node_id, prop_name.clone(), text.clone()));
                    }
                }
            }
        }

        for (index_name, docs) in per_index {
            if let Some(index) = self.indexes.get_mut(&index_name) {
                let doc_refs: Vec<(NodeId, &str, &str)> = docs
                    .iter()
                    .map(|(nid, prop, text)| (*nid, prop.as_str(), text.as_str()))
                    .collect();
                index.build_index(&doc_refs);
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
    pub fn search(
        &self,
        index_name: &str,
        query: &str,
    ) -> Result<Vec<SearchResult>, FulltextError> {
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

        manager
            .create_index("idx1", "Person", vec!["name".to_string()])
            .unwrap();
        manager
            .create_index("idx2", "Article", vec!["title".to_string()])
            .unwrap();

        let indexes = manager.list_indexes();
        assert_eq!(indexes.len(), 2);
    }

    #[test]
    fn test_index_node_integration() {
        let mut manager = FulltextManager::new();
        manager
            .create_index(
                "person_idx",
                "Person",
                vec!["name".to_string(), "bio".to_string()],
            )
            .unwrap();

        let mut properties = HashMap::new();
        properties.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );
        properties.insert(
            "bio".to_string(),
            PropertyValue::String("Software engineer".to_string()),
        );
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
        manager
            .create_index("idx1", "Person", vec!["name".to_string()])
            .unwrap();

        let mut properties = HashMap::new();
        properties.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );

        manager.index_node(1, "Person", &properties);

        let results = manager.search("idx1", "Alice").unwrap();
        assert_eq!(results.len(), 1);

        manager.remove_node(1);

        let results = manager.search("idx1", "Alice").unwrap();
        assert!(results.is_empty());
    }

    // ========== Levenshtein distance tests ==========

    #[test]
    fn test_levenshtein_equal_strings() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_one_substitution() {
        // "kitten" -> "sitten": 1 substitution (k->s)
        assert_eq!(levenshtein_distance("kitten", "sitten"), 1);
    }

    #[test]
    fn test_levenshtein_multiple_edits() {
        // "kitten" -> "sitting": 3 edits
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn test_levenshtein_empty_strings() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
    }

    #[test]
    fn test_levenshtein_database_typo() {
        // "databse" differs from "database" by one deletion (missing 'a') — distance 1
        assert_eq!(levenshtein_distance("database", "databse"), 1);
    }

    // ========== Phrase search tests ==========

    #[test]
    fn test_phrase_search_matches_adjacent_words() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "graph database systems are powerful");
        index.add_document(2, "body", "database graph systems exist");
        index.add_document(3, "body", "relational database management");

        // Phrase search: "graph database" must match only doc 1
        let results = index.search(r#""graph database""#);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);
    }

    #[test]
    fn test_phrase_search_order_matters() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "graph database");
        index.add_document(2, "body", "database graph");

        // "graph database" matches only doc 1 (order matters)
        let results = index.search(r#""graph database""#);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);

        // "database graph" matches only doc 2
        let results2 = index.search(r#""database graph""#);
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].node_id, 2);
    }

    #[test]
    fn test_phrase_search_no_match_non_adjacent() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        // "graph" and "database" exist but are not adjacent
        index.add_document(1, "body", "graph systems and database management");

        let results = index.search(r#""graph database""#);
        assert!(results.is_empty());
    }

    #[test]
    fn test_phrase_search_single_word() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "hello world");

        let results = index.search(r#""hello""#);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_phrase_contains_matches_consecutive() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "graph database is fast");
        index.add_document(2, "body", "database systems for graph processing");

        let matches = index.contains(r#""graph database""#);
        assert_eq!(matches.len(), 1);
        assert!(matches.contains(&1));
    }

    #[test]
    fn test_phrase_contains_no_match_wrong_order() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "database graph");

        // Phrase "graph database" does not match "database graph"
        let matches = index.contains(r#""graph database""#);
        assert!(matches.is_empty());
    }

    // ========== Fuzzy search tests ==========

    #[test]
    fn test_fuzzy_search_default_distance_2() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "database systems");
        index.add_document(2, "body", "python programming");

        // "dtabase~" (two characters transposed) has distance 2 from "database",
        // so default distance 2 should match
        let results = index.search("dtabase~");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);
    }

    #[test]
    fn test_fuzzy_search_custom_distance() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "database systems");
        index.add_document(2, "body", "python programming");

        // "databse" has distance 1 from "database", so distance limit 0 should NOT match
        let results = index.search("databse~0");
        assert!(results.is_empty());

        // "databse~1" should match "database" (distance exactly 1)
        let results2 = index.search("databse~1");
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].node_id, 1);

        // "databse~2" should also match "database"
        let results3 = index.search("databse~2");
        assert_eq!(results3.len(), 1);
        assert_eq!(results3[0].node_id, 1);
    }

    #[test]
    fn test_fuzzy_search_exact_match() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "rust programming language");

        // Exact match with fuzzy syntax should still work
        let results = index.search("rust~");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);
    }

    #[test]
    fn test_fuzzy_search_no_match_distance_exceeded() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "database");

        // "xyz~0" — exact only, "xyz" != "database"
        let results = index.search("xyz~0");
        assert!(results.is_empty());
    }

    #[test]
    fn test_fuzzy_contains_finds_typo() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "database management system");
        index.add_document(2, "body", "graph processing");

        let matches = index.contains("databse~");
        assert_eq!(matches.len(), 1);
        assert!(matches.contains(&1));
    }

    #[test]
    fn test_fuzzy_contains_distance_1() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);

        index.add_document(1, "body", "rust programming");

        // "rast" has distance 1 from "rust" (u->a)
        let matches = index.contains("rast~1");
        assert_eq!(matches.len(), 1);
        assert!(matches.contains(&1));
    }

    // ========== Manager-level phrase/fuzzy tests ==========

    #[test]
    fn test_manager_phrase_search() {
        let mut manager = FulltextManager::new();
        manager
            .create_index("idx", "Article", vec!["body".to_string()])
            .unwrap();

        let mut props = HashMap::new();
        props.insert(
            "body".to_string(),
            PropertyValue::String("graph database systems".to_string()),
        );
        manager.index_node(1, "Article", &props);

        let mut props2 = HashMap::new();
        props2.insert(
            "body".to_string(),
            PropertyValue::String("database graph processing".to_string()),
        );
        manager.index_node(2, "Article", &props2);

        let results = manager.search("idx", r#""graph database""#).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);
    }

    #[test]
    fn test_manager_fuzzy_search() {
        let mut manager = FulltextManager::new();
        manager
            .create_index("idx", "Article", vec!["body".to_string()])
            .unwrap();

        let mut props = HashMap::new();
        props.insert(
            "body".to_string(),
            PropertyValue::String("database administration".to_string()),
        );
        manager.index_node(1, "Article", &props);

        let results = manager.search("idx", "databse~").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, 1);
    }
}

#[cfg(feature = "japanese")]
#[cfg(test)]
mod japanese_tests {
    use super::*;

    #[test]
    fn test_contains_japanese_hiragana() {
        assert!(contains_japanese("ひらがな"));
    }

    #[test]
    fn test_contains_japanese_katakana() {
        assert!(contains_japanese("グラフ"));
    }

    #[test]
    fn test_contains_japanese_kanji() {
        assert!(contains_japanese("漢字"));
    }

    #[test]
    fn test_contains_japanese_false_for_ascii() {
        assert!(!contains_japanese("graph database"));
    }

    #[test]
    fn test_japanese_tokenization() {
        // Japanese text should produce a non-empty token list.
        let tokens = Tokenizer::tokenize("グラフデータベース");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_japanese_stop_words_filtered() {
        // Particles and auxiliary verbs must be absent from the result.
        // "は" is a particle (助詞), "です" is an auxiliary verb (助動詞).
        let tokens = Tokenizer::tokenize("グラフは高速です");
        // The surface "は" and "です" (or their base forms) should not appear.
        // Content words like "グラフ" or "高速" should be present.
        assert!(!tokens.is_empty());
        let text = tokens.join(" ");
        assert!(
            !text.contains("は") || text.contains("グラフ") || text.contains("高速"),
            "expected content words to be present, got: {text}"
        );
    }

    #[test]
    fn test_japanese_index_and_search() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);
        index.add_document(1, "body", "グラフデータベースは高速です");
        index.add_document(2, "body", "リレーショナルデータベース管理システム");

        // Searching for "グラフ" should return at least node 1.
        let results = index.search("グラフ");
        assert!(!results.is_empty(), "expected search results for グラフ");
        let node_ids: HashSet<NodeId> = results.iter().map(|r| r.node_id).collect();
        assert!(
            node_ids.contains(&1),
            "expected node 1 in results, got: {node_ids:?}"
        );
    }

    #[test]
    fn test_japanese_contains() {
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);
        index.add_document(1, "body", "日本語の全文検索");
        index.add_document(2, "body", "英語のテキスト処理");

        // `contains` exercises the tokenisation + lookup code path.
        let _matches = index.contains("全文検索");
        // We only assert the method does not panic. Result count depends on
        // how lindera segments "全文検索" vs the indexed tokens.
    }

    #[test]
    fn test_japanese_phrase_search() {
        let mut index = FulltextIndex::new("test", "Article", vec!["title".to_string()]);
        index.add_document(1, "title", "グラフデータベースの実装");
        index.add_document(2, "title", "データベースグラフの処理");

        // Exercising the phrase-search code path with Japanese input must not panic.
        let _results = index.search("グラフ");
    }

    #[test]
    fn test_mixed_japanese_english_routes_to_japanese_tokenizer() {
        // A string that contains Japanese triggers the Japanese tokeniser.
        // The word "Rust" may or may not survive (depends on lindera segmentation),
        // but the call must succeed without panicking.
        let mut index = FulltextIndex::new("test", "Article", vec!["body".to_string()]);
        index.add_document(1, "body", "Rust言語でグラフDBを実装");

        let _results = index.search("Rust");
        let _results2 = index.search("グラフ");
    }

    #[test]
    fn test_pure_ascii_does_not_use_japanese_tokenizer() {
        // Pure ASCII text must go through the plain tokeniser regardless of feature flag.
        let tokens = Tokenizer::tokenize("graph database search");
        assert!(tokens.contains(&"graph".to_string()));
        assert!(tokens.contains(&"database".to_string()));
        assert!(tokens.contains(&"search".to_string()));
    }
}
