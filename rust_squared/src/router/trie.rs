use std::collections::HashMap;

use http::Method;
use percent_encoding::percent_decode_str;
use smallvec::SmallVec;

use crate::error::RsqError;

use super::PathParams;

#[derive(Default, Clone)]
pub struct TrieNode {
    static_children: HashMap<String, TrieNode>,
    param_child: Option<(String, Box<TrieNode>)>,
    route_indices: HashMap<Method, usize>,
    /// fix(H-4): catch-all wildcard index — set when a `**` segment is the final
    /// segment of an inserted pattern. Matches any remaining path depth during find.
    wildcard_index: Option<usize>,
}

impl TrieNode {
    pub fn insert(
        &mut self,
        pattern: &str,
        method: &Method,
        route_index: usize,
    ) -> Result<(), RsqError> {
        let segments = normalize_path(pattern);
        let mut current = self;
        for segment in segments.iter().copied() {
            // fix(H-4): `**` is a catch-all wildcard; must be the last segment.
            // Store the route index on the current node and stop descending.
            if segment == "**" {
                current.wildcard_index = Some(route_index);
                return Ok(());
            }
            if is_param(segment) {
                let param_name = segment[1..segment.len() - 1].to_string();
                if current.param_child.is_none() {
                    current.param_child = Some((param_name.clone(), Box::<TrieNode>::default()));
                }

                let (existing_name, child) = current
                    .param_child
                    .as_mut()
                    .expect("param child exists after initialization");
                if existing_name != &param_name {
                    return Err(RsqError::bad_request(format!(
                        "conflicting parameter names in route segment: `{}`",
                        segment
                    )));
                }
                current = child;
            } else {
                current = current.static_children.entry(segment.to_string()).or_default();
            }
        }

        if current.route_indices.insert(method.clone(), route_index).is_some() {
            return Err(RsqError::bad_request(format!(
                "route `{}` with method `{}` already registered",
                pattern, method
            )));
        }

        Ok(())
    }

    pub fn find(&self, path: &str, method: &Method) -> Match {
        let segments = normalize_path(path);
        // params is owned here; find_inner mutates it via &mut reference.
        // On a successful match we move it directly — no clone needed (F4).
        let mut params = PathParams::default();
        match self.find_inner(&segments, 0, &mut params, method) {
            Some(FindResult::Found(index)) => Match::Found(index, params),
            Some(FindResult::MethodNotAllowed(methods)) => Match::MethodNotAllowed(methods),
            None => Match::NotFound,
        }
    }

    fn find_inner(
        &self,
        segments: &[&str],
        depth: usize,
        params: &mut PathParams,
        method: &Method,
    ) -> Option<FindResult> {
        // fix(H-4): if this node has a catch-all wildcard, it matches any remaining
        // path — check before exhausting segments so deep paths are handled.
        if let Some(index) = self.wildcard_index {
            return Some(FindResult::Found(index));
        }

        if depth == segments.len() {
            if let Some(index) = self.route_indices.get(method) {
                // Return only the index; params are already in the caller's mut ref.
                return Some(FindResult::Found(*index));
            }
            if !self.route_indices.is_empty() {
                return Some(FindResult::MethodNotAllowed(
                    self.route_indices.keys().cloned().collect(),
                ));
            }
            return None;
        }

        let segment = segments[depth];
        if let Some(child) = self.static_children.get(segment) {
            if let Some(found) = child.find_inner(segments, depth + 1, params, method) {
                return Some(found);
            }
        }

        if let Some((param_name, child)) = &self.param_child {
            let decoded = percent_decode_str(segment).decode_utf8_lossy().to_string();
            params.insert(param_name.clone(), decoded);
            let result = child.find_inner(segments, depth + 1, params, method);
            if result.is_none() {
                params.remove(param_name);
            }
            return result;
        }

        None
    }
}

/// Internal result type for find_inner — params are threaded via &mut ref so
/// we don't carry them in this enum (avoids the clone at every match leaf).
enum FindResult {
    Found(usize),
    MethodNotAllowed(Vec<Method>),
}

pub enum Match {
    Found(usize, PathParams),
    MethodNotAllowed(Vec<Method>),
    NotFound,
}

fn is_param(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}') && segment.len() > 2
}

/// Split a URL path into segments without heap-allocating for paths with ≤8
/// segments (F5). Uses SmallVec to stay on the stack for typical paths.
pub fn normalize_path(path: &str) -> SmallVec<[&str; 8]> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        SmallVec::new()
    } else {
        trimmed.split('/').collect()
    }
}
