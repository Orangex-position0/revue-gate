use std::collections::HashMap;

use crate::domain::knowledge::{KeywordHit, VectorHit};

#[derive(Debug, Clone, PartialEq)]
pub struct RankedVectorHit {
    pub chunk_id: String,
    pub score: f32,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankedKeywordHit {
    pub chunk_id: String,
    pub score: f32,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FusedHit {
    pub chunk_id: String,
    pub score: f32,
    pub vector_score: Option<f32>,
    pub keyword_score: Option<f32>,
    pub vector_rank: Option<usize>,
    pub keyword_rank: Option<usize>,
}

pub fn rank_vector_hits(mut hits: Vec<VectorHit>) -> Vec<RankedVectorHit> {
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    hits.into_iter()
        .enumerate()
        .map(|(index, hit)| RankedVectorHit {
            chunk_id: hit.chunk_id,
            score: hit.score,
            rank: index + 1,
        })
        .collect()
}

pub fn rank_keyword_hits(mut hits: Vec<KeywordHit>) -> Vec<RankedKeywordHit> {
    hits.sort_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    hits.into_iter()
        .enumerate()
        .map(|(index, hit)| RankedKeywordHit {
            chunk_id: hit.chunk_id,
            score: hit.score,
            rank: index + 1,
        })
        .collect()
}

pub fn rrf_fuse(
    vector: Vec<RankedVectorHit>,
    keyword: Vec<RankedKeywordHit>,
    limit: usize,
    rrf_k: f32,
) -> Vec<FusedHit> {
    let mut by_chunk: HashMap<String, FusedHit> = HashMap::new();
    for hit in vector {
        let entry = by_chunk.entry(hit.chunk_id.clone()).or_insert(FusedHit {
            chunk_id: hit.chunk_id,
            score: 0.0,
            vector_score: None,
            keyword_score: None,
            vector_rank: None,
            keyword_rank: None,
        });
        entry.score += 1.0 / (rrf_k + hit.rank as f32);
        entry.vector_score = Some(hit.score);
        entry.vector_rank = Some(hit.rank);
    }
    for hit in keyword {
        let entry = by_chunk.entry(hit.chunk_id.clone()).or_insert(FusedHit {
            chunk_id: hit.chunk_id,
            score: 0.0,
            vector_score: None,
            keyword_score: None,
            vector_rank: None,
            keyword_rank: None,
        });
        entry.score += 1.0 / (rrf_k + hit.rank as f32);
        entry.keyword_score = Some(hit.score);
        entry.keyword_rank = Some(hit.rank);
    }
    let mut fused = by_chunk.into_values().collect::<Vec<_>>();
    fused.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    fused.truncate(limit);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_fuse_prefers_chunks_ranked_by_multiple_channels() {
        let vector = vec![
            RankedVectorHit {
                chunk_id: "a".into(),
                score: 0.90,
                rank: 1,
            },
            RankedVectorHit {
                chunk_id: "b".into(),
                score: 0.80,
                rank: 2,
            },
        ];
        let keyword = vec![
            RankedKeywordHit {
                chunk_id: "b".into(),
                score: -0.10,
                rank: 1,
            },
            RankedKeywordHit {
                chunk_id: "c".into(),
                score: -0.20,
                rank: 2,
            },
        ];

        let fused = rrf_fuse(vector, keyword, 3, 60.0);

        assert_eq!(fused[0].chunk_id, "b");
        assert_eq!(fused[0].vector_rank, Some(2));
        assert_eq!(fused[0].keyword_rank, Some(1));
    }

    #[test]
    fn rrf_fuse_uses_stable_tie_breaker() {
        let vector = vec![
            RankedVectorHit {
                chunk_id: "b".into(),
                score: 0.9,
                rank: 1,
            },
            RankedVectorHit {
                chunk_id: "a".into(),
                score: 0.9,
                rank: 1,
            },
        ];

        let fused = rrf_fuse(vector, vec![], 2, 60.0);

        assert_eq!(
            fused
                .iter()
                .map(|hit| hit.chunk_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
}
