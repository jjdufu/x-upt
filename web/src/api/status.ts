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
  AuthSession,
  BusinessMethodId,
  FrontCategory,
  FrontStatus,
  GlobalSettings,
  MethodStatus,
  SystemCounts,
  SystemStatus,
} from "./types";

export function isBusinessMethodId(value: unknown): value is BusinessMethodId {
  return (
    value === "x_upt.posts.search" ||
    value === "x_upt.photos.render" ||
    value === "x_upt.imgdef.compare"
  );
}

export function isFrontCategory(value: unknown): value is FrontCategory {
  return value === "core" || value === "envelope";
}

export function isAuthSession(value: unknown): value is AuthSession {
  return (
    isRecord(value) && isBoolean(value.auth_required) && isBoolean(value.authenticated)
  );
}

function isAuthResult(value: unknown): value is { authenticated: boolean } {
  return isRecord(value) && isBoolean(value.authenticated);
}

export function isGlobalSettings(value: unknown): value is GlobalSettings {
  return (
    isRecord(value) &&
    isNumber(value.latency_min_ms) &&
    isNumber(value.latency_max_ms) &&
    isNumber(value.token_expire_minutes) &&
    isNumber(value.token_cleanup_seconds) &&
    isNumber(value.token_max_per_client) &&
    isNumber(value.sign_window_seconds) &&
    isNumber(value.sign_nonce_ttl_seconds) &&
    isNumber(value.job_expire_seconds)
  );
}

export function isMethodStatus(value: unknown): value is MethodStatus {
  return (
    isRecord(value) &&
    isBusinessMethodId(value.method) &&
    isString(value.label) &&
    isBoolean(value.enabled) &&
    isBoolean(value.can_enable) &&
    isStringArray(value.missing_preconditions)
  );
}

export function isFrontStatus(value: unknown): value is FrontStatus {
  return (
    isRecord(value) &&
    isString(value.name) &&
    isFrontCategory(value.category) &&
    isString(value.protocol) &&
    isBoolean(value.enabled) &&
    isBoolean(value.running) &&
    isString(value.host) &&
    isNumber(value.port) &&
    isBoolean(value.http2) &&
    isBoolean(value.h2c) &&
    isNumber(value.expire_minutes) &&
    isStringArray(value.auth) &&
    isStringArray(value.supported_auth) &&
    isBoolean(value.tls_enabled) &&
    isBoolean(value.has_tls_cert) &&
    isBoolean(value.has_tls_key) &&
    isBoolean(value.has_tls_client_ca) &&
    isNumber(value.ws_max_conn) &&
    isNumber(value.ws_ping_secs) &&
    isNumber(value.job_workers) &&
    isNumber(value.profile_count) &&
    isBoolean(value.can_enable) &&
    isStringArray(value.missing_preconditions)
  );
}

export function isSystemCounts(value: unknown): value is SystemCounts {
  if (!isRecord(value) || !isRecord(value.images_by_scene)) {
    return false;
  }
  const scenes = value.images_by_scene;
  return (
    isNumber(value.images) &&
    isNumber(scenes.article_cover) &&
    isNumber(scenes.avatar) &&
    isNumber(scenes.banner) &&
    isNumber(value.posts) &&
    isNumber(value.apps) &&
    isNumber(value.envelope_secrets) &&
    isNumber(value.envelope_profiles)
  );
}

export function isSystemStatus(value: unknown): value is SystemStatus {
  return (
    isRecord(value) &&
    isString(value.version) &&
    isString(value.admin_listen) &&
    isSystemCounts(value.counts) &&
    isGlobalSettings(value.settings) &&
    isArrayOf(value.methods, isMethodStatus) &&
    isArrayOf(value.fronts, isFrontStatus)
  );
}

export function fetchAuthSession(): Promise<AuthSession> {
  return requestJson("/api/v1/auth/session", isAuthSession);
}

export function loginAdmin(token: string): Promise<{ authenticated: boolean }> {
  return requestJson("/api/v1/auth/login", isAuthResult, {
    method: "POST",
    body: { token },
  });
}

export function logoutAdmin(): Promise<{ authenticated: boolean }> {
  return requestJson("/api/v1/auth/logout", isAuthResult, {
    method: "POST",
  });
}

export function fetchSystemStatus(): Promise<SystemStatus> {
  return requestJson("/api/v1/status", isSystemStatus);
}

export function fetchSettings(): Promise<GlobalSettings> {
  return requestJson("/api/v1/settings", isGlobalSettings);
}

export function updateSettings(settings: GlobalSettings): Promise<GlobalSettings> {
  return requestJson("/api/v1/settings", isGlobalSettings, {
    method: "PUT",
    body: settings,
  });
}
