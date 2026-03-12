use serde_json::{Value, json};

use crate::{Html, Json};
use crate::router::Route;

pub fn build_spec(routes: &[Route]) -> Value {
    let mut paths = serde_json::Map::new();

    for route in routes {
        let path_item = paths
            .entry(route.pattern().trim_end_matches('/').to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let path_object = path_item
            .as_object_mut()
            .expect("path item should always be an object");

        let parameters = route
            .pattern()
            .split('/')
            .filter(|segment| segment.starts_with('{') && segment.ends_with('}'))
            .map(|segment| {
                let name = segment.trim_start_matches('{').trim_end_matches('}');
                json!({
                    "name": name,
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                })
            })
            .collect::<Vec<_>>();

        path_object.insert(
            route.method().as_str().to_ascii_lowercase(),
            json!({
                "summary": route.meta().summary(),
                "description": route.meta().description(),
                "operationId": route.meta().operation_id(),
                "tags": route.meta().tags(),
                "responses": {
                    "200": { "description": "Successful response" }
                },
                "parameters": parameters
            }),
        );
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "FastRust API",
            "version": "0.1.0"
        },
        "paths": paths
    })
}

pub fn openapi_response(routes: &[Route]) -> Json<Value> {
    Json(build_spec(routes))
}

pub fn swagger_ui_response(spec_path: &str) -> Html {
    Html(format!(
        r##"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <title>FastRust Docs</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
      window.ui = SwaggerUIBundle({{
        url: "{spec_path}",
        dom_id: "#swagger-ui"
      }});
    </script>
  </body>
</html>"##
    ))
}

#[cfg(test)]
mod tests {
    use http::Method;

    use super::build_spec;
    use crate::router::{Route, RouteMeta};

    #[test]
    fn spec_includes_registered_routes() {
        let routes = vec![
            Route::new(Method::GET, "/users/{id}", |_| async { Ok("ok") }),
            Route::new(Method::POST, "/users", |_| async { Ok("ok") }),
        ];

        let spec = build_spec(&routes);
        assert_eq!(spec["openapi"], "3.1.0");
        assert!(spec["paths"]["/users/{id}"]["get"].is_object());
        assert!(spec["paths"]["/users"]["post"].is_object());
    }

    #[test]
    fn spec_includes_route_metadata() {
        let mut meta = RouteMeta::default();
        meta.set_summary("Fetch user");
        meta.set_description("Returns one user by id");
        meta.set_operation_id("getUser");
        meta.add_tag("Users");
        let routes = vec![Route::new(Method::GET, "/users/{id}", |_| async { Ok("ok") }).with_meta(meta)];

        let spec = build_spec(&routes);
        assert_eq!(spec["paths"]["/users/{id}"]["get"]["summary"], "Fetch user");
        assert_eq!(spec["paths"]["/users/{id}"]["get"]["operationId"], "getUser");
        assert_eq!(spec["paths"]["/users/{id}"]["get"]["tags"][0], "Users");
    }
}
