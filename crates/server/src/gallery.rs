//! In-memory gallery of active face templates.
//!
//! For a school (hundreds to a few thousand students) brute-force cosine over
//! the whole gallery is fast and *exact* — no approximate-search recall loss.
//! The cache is refreshed from the DB whenever enrollments change, so matching
//! never touches the database on the hot path.

use db::GalleryRow;
use domain::{cosine_similarity, Embedding};
use std::sync::RwLock;
use uuid::Uuid;

/// One student's templates, grouped so matching can take the per-student best.
#[derive(Clone, Debug)]
pub struct StudentTemplates {
    pub student_id: Uuid,
    pub embeddings: Vec<Embedding>,
}

#[derive(Default)]
pub struct GalleryCache {
    inner: RwLock<Vec<StudentTemplates>>,
}

impl GalleryCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the whole gallery (called after any enrollment change).
    pub fn replace(&self, rows: Vec<GalleryRow>) {
        let mut grouped: std::collections::HashMap<Uuid, Vec<Embedding>> =
            std::collections::HashMap::new();
        for row in rows {
            grouped
                .entry(row.student_id)
                .or_default()
                .push(row.embedding);
        }
        let templates: Vec<StudentTemplates> = grouped
            .into_iter()
            .map(|(student_id, embeddings)| StudentTemplates {
                student_id,
                embeddings,
            })
            .collect();
        let count: usize = templates.iter().map(|t| t.embeddings.len()).sum();
        tracing::info!(
            students = templates.len(),
            templates = count,
            "gallery cache replaced"
        );
        *self.inner.write().expect("gallery lock") = templates;
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().expect("gallery lock").is_empty()
    }

    /// Reload the gallery from the database. Returns the student count.
    /// Errors are logged and swallowed: on a DB hiccup we keep serving the
    /// previous gallery rather than failing closed on every match.
    pub async fn reload(&self, db: &db::Db) -> usize {
        match db::face_repo::load_active_gallery(db).await {
            Ok(rows) => {
                self.replace(rows);
                self.student_count()
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to reload gallery; keeping previous");
                self.student_count()
            }
        }
    }

    pub fn student_count(&self) -> usize {
        self.inner.read().expect("gallery lock").len()
    }

    /// Rank all students by their best-matching template.
    ///
    /// Returns `(student_id, best_similarity)` sorted descending. The caller
    /// only needs the top few for the margin check, but ranking everything is
    /// cheap at this scale and keeps the margin honest.
    pub fn rank(&self, query: &Embedding) -> Vec<(Uuid, f32)> {
        let guard = self.inner.read().expect("gallery lock");
        let mut scored: Vec<(Uuid, f32)> = guard
            .iter()
            .map(|student| {
                let best = student
                    .embeddings
                    .iter()
                    .map(|e| cosine_similarity(query, e))
                    .fold(f32::MIN, f32::max);
                (student.student_id, best)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
    }

    /// Rank excluding one student (used during enrollment duplicate checks).
    pub fn rank_excluding(&self, query: &Embedding, exclude: Uuid) -> Vec<(Uuid, f32)> {
        self.rank(query)
            .into_iter()
            .filter(|(id, _)| *id != exclude)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::EMBEDDING_DIM;

    fn emb(cluster: usize, noise: f32) -> Embedding {
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        let base = (cluster % 32) * 16;
        for i in 0..16 {
            v[(base + i) % EMBEDDING_DIM] = 1.0;
        }
        for (i, slot) in v.iter_mut().enumerate() {
            *slot += ((i as f32) * 0.7 + noise).sin() * 0.05;
        }
        Embedding::new_normalized(v).unwrap()
    }

    #[test]
    fn empty_gallery_ranks_nothing() {
        let g = GalleryCache::new();
        assert!(g.is_empty());
        assert!(g.rank(&emb(0, 0.0)).is_empty());
    }

    #[test]
    fn rank_orders_by_similarity() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let g = GalleryCache::new();
        g.replace(vec![
            GalleryRow {
                student_id: a,
                embedding: emb(0, 0.0),
            },
            GalleryRow {
                student_id: b,
                embedding: emb(10, 0.0),
            },
        ]);
        let ranked = g.rank(&emb(0, 0.01));
        assert_eq!(ranked[0].0, a, "matching cluster should rank first");
        assert!(ranked[0].1 > ranked[1].1);
    }

    #[test]
    fn multiple_templates_take_best() {
        let a = Uuid::from_u128(1);
        let g = GalleryCache::new();
        g.replace(vec![
            GalleryRow {
                student_id: a,
                embedding: emb(20, 0.0),
            },
            GalleryRow {
                student_id: a,
                embedding: emb(0, 0.0),
            },
        ]);
        // Querying cluster 0 should match via the second template.
        let ranked = g.rank(&emb(0, 0.0));
        assert_eq!(ranked.len(), 1, "one row per student");
        assert!(ranked[0].1 > 0.9);
    }

    #[test]
    fn rank_excluding_skips_student() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let g = GalleryCache::new();
        g.replace(vec![
            GalleryRow {
                student_id: a,
                embedding: emb(0, 0.0),
            },
            GalleryRow {
                student_id: b,
                embedding: emb(10, 0.0),
            },
        ]);
        let ranked = g.rank_excluding(&emb(0, 0.0), a);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, b);
    }
}
