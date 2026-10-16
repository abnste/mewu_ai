// SPDX-License-Identifier: MPL-2.0
//! Trusted synthetic invocation hints use the real schema and strict parser.
use super::*;

fn input_with_profile(profile: VisualCoordinateProfile) -> VerifiedVisualInput {
    let mut manifest = tests::input().manifest().clone();
    manifest.targets[0].profile_id = profile.id().into();
    VerifiedVisualInput::new(manifest).unwrap()
}

fn assert_example(scope: &RunScope) -> Value {
    let example = invocation_example(&scope.input).unwrap();
    let definition = scope.definition();
    let description = definition["function"]["description"].as_str().unwrap();
    assert!(description.contains(INVOCATION_GUIDANCE));
    assert!(description.contains(&format!("Complete example arguments: {example}")));
    let validator = jsonschema::validator_for(&definition["function"]["parameters"]).unwrap();
    {
        assert!(validator.is_valid(&example));
        assert!(scope.parse_plan(&example.to_string()).is_ok());
        assert_eq!(example.as_object().unwrap().len(), 1);
        assert_eq!(example["groups"].as_array().unwrap().len(), 1);
        let group = &example["groups"][0];
        let target = scope
            .input
            .manifest()
            .targets
            .iter()
            .find(|target| group["target"] == target.handle)
            .unwrap();
        assert!(group.get("targetHandle").is_none());
        let object = &group["objects"][0];
        assert_eq!(object["kind"], "text");
        assert_eq!(object["text"], "A");
        assert!(object["fontSize"].is_number());
        assert!(object["strokeWidth"].is_number());
        assert!(object["points"][0]["x"].is_number());
        assert!(object["points"][0]["y"].is_number());
        if VisualCoordinateProfile::from_id(&target.profile_id)
            .unwrap()
            .normalized()
        {
            assert_eq!(group["coordinateSpace"], "normalized1000");
            assert_eq!(group["styleUnit"], "encodedPixels");
            assert_eq!(object["anchor"], "topLeft");
        } else {
            assert!(group.get("coordinateSpace").is_none());
            assert!(group.get("styleUnit").is_none());
            assert!(object.get("anchor").is_none());
        }
    }
    example
}

#[test]
fn trusted_examples_obey_the_real_normalized_and_both_pixel_contracts() {
    for profile in [
        VisualCoordinateProfile::ExplicitNormalized1000V2,
        VisualCoordinateProfile::EncodedPixelsBestEffortV1,
        VisualCoordinateProfile::ClaudeNativePixelsV1,
    ] {
        let scope = tests::scope(input_with_profile(profile));
        assert_example(&scope);
    }
}

#[test]
fn multiple_target_examples_keep_identity_units_and_existing_rejections() {
    let mut manifest = input_with_profile(VisualCoordinateProfile::EncodedPixelsBestEffortV1)
        .manifest()
        .clone();
    let mut second = manifest.targets[0].clone();
    second.handle = uuid::Uuid::new_v4().to_string();
    second.attachment_id = uuid::Uuid::new_v4().to_string();
    second.attachment_ordinal = 1;
    second.region_id = uuid::Uuid::new_v4().to_string();
    second.profile_id = VisualCoordinateProfile::ExplicitNormalized1000V2
        .id()
        .into();
    manifest.targets.push(second);
    let mut example_targets = Vec::new();
    for _ in 0..2 {
        let scope = tests::scope(VerifiedVisualInput::new(manifest.clone()).unwrap());
        let example = assert_example(&scope);
        assert_eq!(example["groups"][0]["target"], manifest.targets[0].handle);
        example_targets.push(example["groups"][0]["target"].clone());
        let definition = scope.definition();
        let validator = jsonschema::validator_for(&definition["function"]["parameters"]).unwrap();
        for mutate in [0, 1, 2, 3] {
            let mut bad = example.clone();
            match mutate {
                0 => bad["groups"][0]["target"] = json!(uuid::Uuid::new_v4().to_string()),
                1 => {
                    bad["groups"][0].as_object_mut().unwrap().remove("target");
                }
                2 => bad["groups"][0]["objects"][0]["fontSize"] = json!("16"),
                _ => bad["untrusted-field"] = json!("private-user-text"),
            }
            assert!(!validator.is_valid(&bad));
            assert!(scope.parse_plan(&bad.to_string()).is_err());
        }
        manifest.targets.reverse();
    }
    assert_ne!(example_targets[0], example_targets[1]);
}

#[test]
fn invocation_hints_project_no_source_identity_or_user_content_and_skip_invalid_examples() {
    let scope = tests::scope(tests::normalized_input());
    let encoded = assert_example(&scope).to_string();
    let target = &scope.input.manifest().targets[0];
    for private in [
        &scope.origin.scene_id,
        &target.attachment_id,
        &target.region_id,
        &target.source.background_id,
        &target.source.effective_source_id,
        &scope.origin.manifest_sha256,
    ] {
        assert!(!encoded.contains(private.as_str()));
    }
    let mut manifest = tests::input().manifest().clone();
    manifest.targets[0].encoded.width = 1;
    manifest.targets[0].encoded.height = 1;
    let scope = tests::scope(VerifiedVisualInput::new(manifest).unwrap());
    assert!(invocation_example(&scope.input).is_none());
}
