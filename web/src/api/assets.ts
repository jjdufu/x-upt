import {
  isArrayOf,
  isBoolean,
  isNumber,
  isRecord,
  isString,
  isStringArray,
  requestJson,
} from "./http";
import type {
  AppMeta,
  AppUpsertPayload,
  ImageMeta,
  ImageScene,
  PostInput,
  PostRecord,
  PostsPage,
} from "./types";

export function isImageScene(value: unknown): value is ImageScene {
  return value === "article_cover" || value === "avatar" || value === "banner";
}

export function isImageMeta(value: unknown): value is ImageMeta {
  return (
    isRecord(value) &&
    isString(value.id) &&
    isString(value.filename) &&
    isImageScene(value.scene) &&
    isString(value.mime) &&
    isNumber(value.width) &&
    isNumber(value.height) &&
    isNumber(value.size) &&
    isString(value.ahash) &&
    isString(value.created_at)
  );
}

function isImageListResponse(value: unknown): value is { items: ImageMeta[] } {
  return isRecord(value) && isArrayOf(value.items, isImageMeta);
}

function isDeletedResponse(value: unknown): value is { deleted: boolean } {
  return isRecord(value) && isBoolean(value.deleted);
}

export function isPostRecord(value: unknown): value is PostRecord {
  return (
    isRecord(value) &&
    isNumber(value.id) &&
    isNumber(value.userId) &&
    isString(value.title) &&
    isString(value.body)
  );
}

export function isPostsPage(value: unknown): value is PostsPage {
  return (
    isRecord(value) &&
    isArrayOf(value.items, isPostRecord) &&
    isNumber(value.total) &&
    isNumber(value.page) &&
    isNumber(value.page_size)
  );
}

export function isAppMeta(value: unknown): value is AppMeta {
  return (
    isRecord(value) &&
    isString(value.client_id) &&
    isString(value.name) &&
    isBoolean(value.has_secret) &&
    isString(value.secret_preview) &&
    isBoolean(value.has_rsa_public_pem) &&
    isStringArray(value.methods) &&
    isString(value.created_at)
  );
}

function isAppListResponse(value: unknown): value is { items: AppMeta[] } {
  return isRecord(value) && isArrayOf(value.items, isAppMeta);
}

export function imageRawUrl(id: string): string {
  return `/api/v1/images/${encodeURIComponent(id)}/raw`;
}

export function fetchImages(): Promise<{ items: ImageMeta[] }> {
  return requestJson("/api/v1/images", isImageListResponse);
}

export function uploadImages(
  scene: ImageScene,
  files: File[],
): Promise<{ items: ImageMeta[] }> {
  const form = new FormData();
  form.append("scene", scene);
  for (const file of files) {
    form.append("file", file, file.name);
  }
  return requestJson("/api/v1/images", isImageListResponse, {
    method: "POST",
    formData: form,
  });
}

export function deleteImage(id: string): Promise<{ deleted: boolean }> {
  return requestJson(
    `/api/v1/images/${encodeURIComponent(id)}`,
    isDeletedResponse,
    { method: "DELETE" },
  );
}

export function fetchPosts(
  page = 1,
  pageSize = 50,
  keyword = "",
): Promise<PostsPage> {
  const params = new URLSearchParams({
    page: String(page),
    page_size: String(pageSize),
    keyword,
  });
  return requestJson(`/api/v1/posts?${params.toString()}`, isPostsPage);
}

export function createPost(input: PostInput): Promise<PostRecord> {
  return requestJson("/api/v1/posts", isPostRecord, {
    method: "POST",
    body: input,
  });
}

export function importPosts(items: PostInput[]): Promise<Record<string, unknown>> {
  return requestJson("/api/v1/posts/import", isRecord, {
    method: "POST",
    body: { items },
  });
}

export function updatePost(id: number, input: PostInput): Promise<PostRecord> {
  return requestJson(`/api/v1/posts/${encodeURIComponent(String(id))}`, isPostRecord, {
    method: "PUT",
    body: input,
  });
}

export function deletePost(id: number): Promise<{ deleted: boolean }> {
  return requestJson(
    `/api/v1/posts/${encodeURIComponent(String(id))}`,
    isDeletedResponse,
    { method: "DELETE" },
  );
}

export function fetchApps(): Promise<{ items: AppMeta[] }> {
  return requestJson("/api/v1/apps", isAppListResponse);
}

export function createApp(payload: AppUpsertPayload): Promise<AppMeta> {
  return requestJson("/api/v1/apps", isAppMeta, {
    method: "POST",
    body: payload,
  });
}

export function updateApp(
  clientId: string,
  payload: AppUpsertPayload,
): Promise<AppMeta> {
  return requestJson(
    `/api/v1/apps/${encodeURIComponent(clientId)}`,
    isAppMeta,
    { method: "PUT", body: payload },
  );
}

export function deleteApp(clientId: string): Promise<{ deleted: boolean }> {
  return requestJson(
    `/api/v1/apps/${encodeURIComponent(clientId)}`,
    isDeletedResponse,
    { method: "DELETE" },
  );
}
