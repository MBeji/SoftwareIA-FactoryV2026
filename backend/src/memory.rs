//! Vector / memory store: what the factory learns during a run (architecture decisions,
//! lessons from failed attempts) and recalls when a similar problem shows up.
//!
//! The MVP embeds text with a dependency-free hashing vectoriser and keeps vectors in
//! memory. The [`Embedder`] trait is the seam for real embeddings (via the LiteLLM
//! `/embeddings` endpoint) and a real vector database (Qdrant, pgvector).

use std::sync::RwLock;

use serde::{Deserialize, Serialize};

pub const DIMENSIONS: usize = 256;

pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Bag-of-words hashing trick (FNV-1a over lower-cased word unigrams and bigrams),
/// L2-normalised. Deterministic and offline; good enough to recall near-duplicates.
pub struct HashingEmbedder;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl Embedder for HashingEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let lower = text.to_lowercase();
        let words: Vec<&str> = lower
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| w.len() > 2)
            .collect();
        let mut vector = vec![0f32; DIMENSIONS];
        for (i, word) in words.iter().enumerate() {
            vector[(fnv1a(word.as_bytes()) % DIMENSIONS as u64) as usize] += 1.0;
            if let Some(next) = words.get(i + 1) {
                let bigram = format!("{word} {next}");
                vector[(fnv1a(bigram.as_bytes()) % DIMENSIONS as u64) as usize] += 0.5;
            }
        }
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            vector.iter_mut().for_each(|v| *v /= norm);
        }
        vector
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    /// Scope of the memory, usually the run id (or a project id for cross-run recall).
    pub scope: String,
    /// `lesson`, `architecture`, `decision`…
    pub kind: String,
    pub text: String,
}

struct Entry {
    item: MemoryItem,
    vector: Vec<f32>,
}

pub struct MemoryStore {
    embedder: Box<dyn Embedder>,
    entries: RwLock<Vec<Entry>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new(Box::new(HashingEmbedder))
    }
}

impl MemoryStore {
    pub fn new(embedder: Box<dyn Embedder>) -> Self {
        Self {
            embedder,
            entries: RwLock::new(Vec::new()),
        }
    }

    pub fn add(&self, scope: &str, kind: &str, text: &str) {
        let vector = self.embedder.embed(text);
        let item = MemoryItem {
            scope: scope.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
        };
        self.entries
            .write()
            .expect("memory poisoned")
            .push(Entry { item, vector });
    }

    /// The `k` most similar memories of a scope (optionally of one kind), best first.
    pub fn search(
        &self,
        scope: &str,
        kind: Option<&str>,
        query: &str,
        k: usize,
    ) -> Vec<(f32, MemoryItem)> {
        let q = self.embedder.embed(query);
        let entries = self.entries.read().expect("memory poisoned");
        let mut scored: Vec<(f32, MemoryItem)> = entries
            .iter()
            .filter(|e| e.item.scope == scope && kind.is_none_or(|k| e.item.kind == k))
            .map(|e| {
                (
                    e.vector.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>(),
                    e.item.clone(),
                )
            })
            .filter(|(score, _)| *score > 0.05)
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(k);
        scored
    }

    pub fn len(&self) -> usize {
        self.entries.read().expect("memory poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recalls_the_most_similar_memory_within_a_scope() {
        let memory = MemoryStore::default();
        memory.add(
            "run-1",
            "lesson",
            "The expiry check was missing: compare the exp claim with the current time",
        );
        memory.add(
            "run-1",
            "lesson",
            "Use hmac.compare_digest to compare signatures in constant time",
        );
        memory.add(
            "run-2",
            "lesson",
            "The expiry check was missing in another run",
        );

        let hits = memory.search(
            "run-1",
            Some("lesson"),
            "token expiry claim is not verified",
            1,
        );
        assert_eq!(hits.len(), 1);
        assert!(hits[0].1.text.contains("expiry check"));
        assert!(memory.search("run-3", None, "expiry", 3).is_empty());
        assert!(memory
            .search("run-1", Some("architecture"), "expiry", 3)
            .is_empty());
    }
}
