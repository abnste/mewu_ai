// SPDX-License-Identifier: MPL-2.0
//! Observed synthetic MiniMax response; no credentials, network or UI.
use super::*;

#[test]
fn observed_table_array_wrappers_decode_without_changing_cells_or_weakening_validation() {
    let input = super::tests::normalized_input();
    let scope = super::tests::scope(input.clone()).for_connection(&profile());
    let wire = json!({"groups":[{"target":input.manifest().targets[0].handle,
        "coordinateSpace":"normalized1000","styleUnit":"encodedPixels","objects":[{
        "kind":"table","points":[{"x":690,"y":775},{"x":975,"y":940}],
        "table":{"version":"1","header":{"item":["x","y"]},
        "rows":{"item":[{"item":["1","2"]},{"item":["2","4"]}]},
        "align":{"item":["left","right"]}}}]}]});
    assert!(parse_plan(&wire.to_string(), &input).is_err());
    let plan = scope.parse_plan(&wire.to_string()).unwrap();
    let ParsedObject::Rich { content: RichContent::Table { header, rows, .. }, .. } = &plan.groups[0].objects[0] else {
        panic!("expected a validated table");
    };
    assert_eq!(header, &["x", "y"]);
    assert_eq!(rows, &[vec!["1", "2"], vec!["2", "4"]]);
    for (path, value) in [
        ("/table/version", json!("2")),
        ("/table/version", json!("1.0")),
        ("/table/align", json!({"item":"left"})),
        ("/table/align", json!({"item":["left","right"],"extra":true})),
        ("/table/align", json!({"item":["left"]})),
        ("/table/header", json!({"item":[1,2]})),
        ("/table/rows", json!({"item":[{"item":["1"]}]})),
        ("/table/rows", json!({"item":[{"item":["1","2"],"extra":true}]})),
        ("/table/rows", json!({"item":[{"item":[1,2]}]})),
    ] {
        let mut bad = wire.clone();
        *bad["groups"][0]["objects"][0].pointer_mut(path).unwrap() = value;
        assert!(scope.parse_plan(&bad.to_string()).is_err(), "accepted {path}");
    }
}

fn profile() -> mewu_core::ConnectionProfile {
    mewu_core::ConnectionProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        provider_id: "minimax-cn".into(),
        base_url: "https://api.minimax.cn/v1".into(),
        model: "MiniMax-M3".into(),
        has_key: false,
        credential_id: None,
        revision: 1,
        advanced: Default::default(),
    }
}
fn observed(input: &VerifiedVisualInput) -> Value {
    json!({"groups":[{"target":input.manifest().targets[0].handle,
        "coordinateSpace":"normalized1000","styleUnit":"encodedPixels","objects":[
        {"anchor":"topLeft","color":"#000000","fontSize":"50","kind":"text",
         "points":{"item":{"x":"488","y":"130"}},"text":"4"},
        {"color":"#ff0000","kind":"rect","points":{"item":[{"x":"700","y":"467"},{"x":"860","y":"595"}]},"strokeWidth":"3"}
    ]}]})
}

#[test]
fn observed_minimax_wire_maps_to_the_same_validated_geometry_without_changing_text() {
    let scope = super::tests::scope(super::tests::normalized_input()).for_connection(&profile());
    let wire = observed(&scope.input);
    assert!(parse_plan(&wire.to_string(), &scope.input).is_err());
    let decoded = vector_batch(scope.parse_plan(&wire.to_string()).unwrap()).unwrap();
    let mut canonical = wire;
    canonical["groups"][0]["objects"][0]["fontSize"] = json!(50);
    canonical["groups"][0]["objects"][0]["points"] = json!([{"x":488,"y":130}]);
    canonical["groups"][0]["objects"][1]["strokeWidth"] = json!(3);
    canonical["groups"][0]["objects"][1]["points"] = json!([{"x":700,"y":467},{"x":860,"y":595}]);
    let expected = parse_batch(&canonical.to_string(), &scope.input).unwrap();
    for (actual, expected) in decoded.groups[0]
        .drawings
        .iter()
        .zip(&expected.groups[0].drawings)
    {
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.points, expected.points);
        assert_eq!(actual.font_size, expected.font_size);
        assert_eq!(actual.stroke_width, expected.stroke_width);
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.color, expected.color);
    }
    assert_eq!(decoded.groups[0].drawings.len(), 2);
    assert_eq!(decoded.groups[0].drawings[0].text.as_deref(), Some("4"));
}

#[test]
fn minimax_wire_keeps_field_units_target_numeric_and_point_limits_fail_closed() {
    let input = super::tests::normalized_input();
    for model in [
        "MiniMax-M3",
        "MiniMax-M3.1-Flash-Preview",
        "MiniMax-M3.1-Flash",
    ] {
        let mut connection = profile();
        connection.model = model.into();
        let scope = super::tests::scope(input.clone()).for_connection(&connection);
        let original = observed(&input);
        for path in [
            "/groups/0/objects/0/fontSize",
            "/groups/0/objects/1/strokeWidth",
            "/groups/0/objects/0/points/item/x",
        ] {
            for invalid in [
                "NaN", "Infinity", "1e9999", "01", "+1", "0x10", "50px", " 50", "50 ", "",
            ] {
                let mut bad = original.clone();
                *bad.pointer_mut(path).unwrap() = json!(invalid);
                assert!(
                    scope.parse_plan(&bad.to_string()).is_err(),
                    "accepted {path}/{invalid}"
                );
            }
        }
        let mutations = [
            ("/groups/0/target", json!(uuid::Uuid::new_v4().to_string())),
            ("/groups/0/coordinateSpace", json!("pixels")),
            ("/groups/0/styleUnit", json!(null)),
            ("/groups/0/objects/0/fontSize", json!("1000")),
            ("/groups/0/objects/0/points/item/x", json!("1001")),
            ("/groups/0/objects/1/strokeWidth", json!("0")),
            (
                "/groups/0/objects/0/points",
                json!({"item":{"x":"10","y":"10"},"path":"spoof"}),
            ),
            (
                "/groups/0/objects/1/points",
                json!({"item":{"x":"10","y":"10"}}),
            ),
            (
                "/groups/0/objects/0/points",
                json!({"item":[{"x":"10","y":"10","path":"spoof"}]}),
            ),
        ];
        for (path, value) in mutations {
            let mut bad = original.clone();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(
                scope.parse_plan(&bad.to_string()).is_err(),
                "accepted {path}"
            );
        }
        for field in ["path", "id", "origin", "sceneId", "layoutId"] {
            let mut bad = original.clone();
            bad["groups"][0]["objects"][0][field] = json!("spoof");
            assert!(scope.parse_plan(&bad.to_string()).is_err());
        }
        let duplicated = original.to_string().replacen("{", "{\"groups\":[],", 1);
        assert!(scope.parse_plan(&duplicated).is_err());
        assert!(scope.parse_plan(&" ".repeat(32 * 1024 + 1)).is_err());
    }
}

#[test]
fn minimax_wire_is_bound_to_the_preflight_model_protocol_and_official_endpoint() {
    let input = super::tests::normalized_input();
    let wire = observed(&input).to_string();
    for model in [
        "MiniMax-M3",
        "MiniMax-M3.1-Flash-Preview",
        "MiniMax-M3.1-Flash",
    ] {
        for address in [
            "https://api.minimax.cn/v1",
            "https://api.minimaxi.com/v1",
            "https://api.minimax.io/v1",
            "https://api.minimax.cn:443/v1",
        ] {
            let mut p = profile();
            p.model = model.into();
            p.base_url = address.into();
            assert!(super::tests::scope(input.clone())
                .for_connection(&p)
                .parse_plan(&wire)
                .is_ok());
        }
        for address in [
            "http://localhost/v1",
            "http://api.minimax.cn/v1",
            "https://api.minimax.cn.example/v1",
            "https://example.com/v1",
            "https://api.minimax.cn/custom",
            "https://api.minimax.cn:444/v1",
        ] {
            let mut p = profile();
            p.model = model.into();
            p.base_url = address.into();
            assert!(super::tests::scope(input.clone())
                .for_connection(&p)
                .parse_plan(&wire)
                .is_err());
        }
        for protocol in [
            mewu_core::ConnectionProtocol::AnthropicMessages,
            mewu_core::ConnectionProtocol::OpenAiResponses,
        ] {
            let mut p = profile();
            p.model = model.into();
            p.advanced.protocol = protocol;
            assert!(super::tests::scope(input.clone())
                .for_connection(&p)
                .parse_plan(&wire)
                .is_err());
        }
    }
    for model in [
        "other-model",
        "MiniMax-M3.1-Flash-Preview-extra",
        "MiniMax-M3.1-Flash-extra",
        "custom-MiniMax-M3.1-Flash-Preview",
        "minimax-m3.1-flash-preview",
    ] {
        let mut p = profile();
        p.model = model.into();
        assert!(super::tests::scope(input.clone())
            .for_connection(&p)
            .parse_plan(&wire)
            .is_err());
    }
}

#[test]
fn flash_preview_bounded_wire_matches_canonical_geometry_without_general_type_coercion() {
    let input = super::tests::normalized_input();
    let mut p = profile();
    p.model = "MiniMax-M3.1-Flash-Preview".into();
    let scope = super::tests::scope(input).for_connection(&p);
    let wire = observed(&scope.input);
    assert!(parse_plan(&wire.to_string(), &scope.input).is_err());
    let decoded = vector_batch(scope.parse_plan(&wire.to_string()).unwrap()).unwrap();
    let mut canonical = wire;
    canonical["groups"][0]["objects"][0]["fontSize"] = json!(50);
    canonical["groups"][0]["objects"][0]["points"] = json!([{"x":488,"y":130}]);
    canonical["groups"][0]["objects"][1]["strokeWidth"] = json!(3);
    canonical["groups"][0]["objects"][1]["points"] = json!([{"x":700,"y":467},{"x":860,"y":595}]);
    let expected = parse_batch(&canonical.to_string(), &scope.input).unwrap();
    for (actual, expected) in decoded.groups[0]
        .drawings
        .iter()
        .zip(&expected.groups[0].drawings)
    {
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.points, expected.points);
        assert_eq!(actual.font_size, expected.font_size);
        assert_eq!(actual.stroke_width, expected.stroke_width);
        assert_eq!(actual.text, expected.text);
    }
}
