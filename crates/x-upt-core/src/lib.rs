#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]
//! Pure domain models, precondition guards, deterministic business services, and API documentation models for `x-upt`.

pub mod docs_model;
pub mod domain;
pub mod error;
pub mod imgs;
pub mod logger_types;
pub mod preconditions;
pub mod services;

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use crate::domain::{AuthedApp, BusinessMethod, METHOD_POSTS_SEARCH, MethodsState};
    use crate::imgs::{PhotoFormat, PhotoScene, average_hash, hamming_distance};
    use crate::logger_types::{omit_logged_images, redact_gql_image_literals, redact_query};
    use crate::preconditions::{SceneCounts, guard_delete_post, missing_for_method};
    use crate::services::{
        Call, CoreDataset, ImgdefRequest, PhotosRequest, Post, PostSort, PostsRequest,
    };

    #[test]
    fn precondition_blocks_empty_images_and_posts() -> Result<(), Box<dyn std::error::Error>> {
        let empty = SceneCounts::default();
        assert!(!missing_for_method(BusinessMethod::PhotosRender, empty, 0).is_empty());
        assert!(!missing_for_method(BusinessMethod::ImgdefCompare, empty, 0).is_empty());
        assert!(!missing_for_method(BusinessMethod::PostsSearch, empty, 0).is_empty());

        let ready = SceneCounts {
            article_cover: 1,
            avatar: 1,
            banner: 1,
        };
        assert!(missing_for_method(BusinessMethod::PhotosRender, ready, 1).is_empty());
        assert!(missing_for_method(BusinessMethod::ImgdefCompare, ready, 1).is_empty());
        assert!(missing_for_method(BusinessMethod::PostsSearch, ready, 1).is_empty());

        let active_posts = MethodsState {
            posts_search: true,
            photos_render: false,
            imgdef_compare: false,
        };
        assert!(guard_delete_post(1, active_posts).is_err());
        assert!(guard_delete_post(2, active_posts).is_ok());
        Ok(())
    }

    #[test]
    fn deterministic_posts_photos_and_imgdef() -> Result<(), Box<dyn std::error::Error>> {
        let mut img_buf = image::RgbImage::new(16, 16);
        for (x, y, px) in img_buf.enumerate_pixels_mut() {
            *px = image::Rgb([u8::try_from(x * 10)?, u8::try_from(y * 10)?, 128]);
        }
        let dyn_img = image::DynamicImage::ImageRgb8(img_buf);
        let mut png_bytes = Vec::new();
        dyn_img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )?;
        let hash = average_hash(&dyn_img).ok_or("hash failed")?;
        assert_eq!(hamming_distance(hash, hash), 0);

        let rt_img = crate::imgs::RuntimeImage {
            meta: crate::imgs::ImageMeta {
                id: "1".into(),
                filename: "sample.png".into(),
                scene: PhotoScene::ArticleCover,
                mime: "image/png".into(),
                width: 16,
                height: 16,
                size: u64::try_from(png_bytes.len())?,
                ahash: format!("{hash:016x}"),
                created_at: "2026-01-01T00:00:00Z".into(),
            },
            ahash_u64: hash,
            bytes: png_bytes.clone(),
        };

        let mut scene_images = HashMap::new();
        scene_images.insert(PhotoScene::ArticleCover, vec![rt_img]);
        let mut image_hashes = HashMap::new();
        image_hashes.insert("sample.png".to_string(), hash);

        let ds = CoreDataset {
            methods: MethodsState {
                posts_search: true,
                photos_render: true,
                imgdef_compare: true,
            },
            posts: vec![Post {
                user_id: 6,
                id: 18,
                title: "gRPC streaming".into(),
                body: "deterministic test".into(),
            }],
            scene_images,
            image_hashes,
        };

        let app = AuthedApp {
            client_id: "demo".into(),
            methods: HashSet::from([METHOD_POSTS_SEARCH.to_string()]),
        };
        let out = ds.dispatch(Call::from_app(
            &app,
            METHOD_POSTS_SEARCH.to_string(),
            serde_json::to_value(PostsRequest {
                keyword: "gRPC".into(),
                user_id: 6,
                page: 1,
                page_size: 10,
                sort: PostSort::IdDesc,
            })?,
        ))?;
        assert_eq!(
            out.get("total").and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let photo = crate::services::render_photo(
            &PhotosRequest {
                scene: PhotoScene::ArticleCover,
                width: 32,
                height: 32,
                format: PhotoFormat::Png,
            },
            ds.scene_images
                .get(&PhotoScene::ArticleCover)
                .map_or(&[], Vec::as_slice),
        )?;
        assert_eq!(photo.image.filename, "sample.png");

        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
        let cmp = crate::services::compare_image(
            &ImgdefRequest {
                filename: "sample.png".into(),
                image: b64,
                threshold: 0,
            },
            &ds.image_hashes,
        )?;
        assert!(cmp.matched);
        assert_eq!(cmp.distance, 0);
        Ok(())
    }

    #[test]
    fn redaction_strips_secrets_and_images() -> Result<(), Box<dyn std::error::Error>> {
        let q = redact_query(Some("company_id=acme&image=rawb64&city=hz")).ok_or("missing")?;
        assert!(q.contains("company_id=***"));
        assert!(q.contains("image=***"));
        assert!(q.contains("city=hz"));

        let mut payload = serde_json::json!({
            "filename": "a.png",
            "image": "AAAA",
            "query": "mutation { imgdefCompare(image: \"AAAA\", filename: \"a.png\") { matched } }",
            "nested": { "base64": "BBBB" }
        });
        omit_logged_images(&mut payload);
        assert!(payload.get("image").is_none());
        assert!(
            payload
                .get("nested")
                .and_then(|v| v.get("base64"))
                .is_none()
        );
        let gql = "{ photosRender { image { filename base64 } } }";
        assert_eq!(redact_gql_image_literals(gql), gql);
        Ok(())
    }
}
