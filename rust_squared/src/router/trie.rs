use std::collections::HashMap;

use http::Method;
use percent_encoding::percent_decode_str;

use crate::error::RsqError;

use super::PathParams;

#[derive(Default)]
pub struct TrieNode {
    static_children: HashMap<String, TrieNode>,
    param_child: Option<(String, Box<TrieNode>)>,
    route_indices: HashMap<Method, usize>,
}

impl TrieNode {
    pub fn insert(
        &mut self,
        pattern: &str,
        method: Method,
        route_index: usize,
    ) -> Result<(), RsqError> {
        let segments = normalize_path(pattern);
        let mut current = self;
        for segment in segments {
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
        let mut params = PathParams::default();
        match self.find_inner(&segments, 0, &mut params, method) {
            Some(Match::Found(index, found_params)) => Match::Found(index, found_params),
            Some(Match::MethodNotAllowed(methods)) => Match::MethodNotAllowed(methods),
            Some(Match::NotFound) | None => Match::NotFound,
        }
    }

    fn find_inner(
        &self,
        segments: &[&str],
        depth: usize,
        params: &mut PathParams,
        method: &Method,
    ) -> Option<Match> {
        if depth == segments.len() {
            if let Some(index) = self.route_indices.get(method) {
                return Some(Match::Found(*index, params.clone()));
            }
            if !self.route_indices.is_empty() {
                return Some(Match::MethodNotAllowed(
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

pub enum Match {
    Found(usize, PathParams),
    MethodNotAllowed(Vec<Method>),
    NotFound,
}

fn is_param(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}') && segment.len() > 2
}

pub fn normalize_path(path: &str) -> Vec<&str> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.split('/').collect()
    }
}
