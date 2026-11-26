pub const METHOD_POSTS_SEARCH: &str = "upupt.posts.search";
pub const METHOD_PHOTOS_RENDER: &str = "upupt.photos.render";
pub const METHOD_IMGDEF_COMPARE: &str = "upupt.imgdef.compare";

#[must_use]
pub fn is_known_method(method: &str) -> bool {
    matches!(
        method,
        METHOD_POSTS_SEARCH | METHOD_PHOTOS_RENDER | METHOD_IMGDEF_COMPARE
    )
}

#[must_use]
pub fn known_methods() -> [&'static str; 3] {
    [
        METHOD_POSTS_SEARCH,
        METHOD_PHOTOS_RENDER,
        METHOD_IMGDEF_COMPARE,
    ]
}
