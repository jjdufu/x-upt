import type { ApiErrorBody } from "./types";

export type Guard<T> = (value: unknown) => value is T;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isString(value: unknown): value is string {
  return typeof value === "string";
}

export function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

export function isOptionalNullableString(value: unknown): value is string | null | undefined {
  return value === undefined || value === null || typeof value === "string";
}

export function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

export function isArrayOf<T>(value: unknown, itemGuard: Guard<T>): value is T[] {
  return Array.isArray(value) && value.every((item) => itemGuard(item));
}

export function isApiErrorBody(value: unknown): value is ApiErrorBody {
  if (!isRecord(value)) {
    return false;
  }
  if (!isString(value.error) || !isString(value.message)) {
    return false;
  }
  if (value.missing !== undefined && !isStringArray(value.missing)) {
    return false;
  }
  return true;
}

export class ApiRequestError extends Error {
  readonly status: number;
  readonly code: string;
  readonly missing: string[];

  constructor(status: number, code: string, message: string, missing: string[] = []) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    this.code = code;
    this.missing = missing;
  }
}

export function parseApiError(status: number, payload: unknown): ApiRequestError {
  if (isApiErrorBody(payload)) {
    return new ApiRequestError(status, payload.error, payload.message, payload.missing ?? []);
  }
  if (isRecord(payload) && isString(payload.message)) {
    const code = isString(payload.error) ? payload.error : `http_${status}`;
    const missing = isStringArray(payload.missing) ? payload.missing : [];
    return new ApiRequestError(status, code, payload.message, missing);
  }
  return new ApiRequestError(status, `http_${status}`, `HTTP ${status}`);
}

async function readResponseJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (text.trim().length === 0) {
    return {};
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw new ApiRequestError(response.status, "invalid_json", "Invalid JSON payload");
  }
}

export interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE";
  body?: unknown;
  formData?: FormData;
}

export async function requestJson<T>(
  path: string,
  guard: Guard<T>,
  options: RequestOptions = {},
): Promise<T> {
  const method = options.method ?? "GET";
  const headers: Record<string, string> = {
    Accept: "application/json",
  };
  let bodyInit: BodyInit | undefined;
  if (options.formData !== undefined) {
    bodyInit = options.formData;
  } else if (options.body !== undefined) {
    headers["Content-Type"] = "application/json";
    bodyInit = JSON.stringify(options.body);
  }

  const response = await fetch(path, {
    method,
    headers,
    body: bodyInit,
    credentials: "same-origin",
  });

  const payload = await readResponseJson(response);
  if (!response.ok) {
    throw parseApiError(response.status, payload);
  }
  if (!guard(payload)) {
    throw new ApiRequestError(response.status, "schema_mismatch", `Response mismatch on ${path}`);
  }
  return payload;
}

export async function requestText(path: string): Promise<string> {
  const response = await fetch(path, {
    method: "GET",
    credentials: "same-origin",
  });
  if (!response.ok) {
    const payload = await readResponseJson(response).catch(() => null);
    throw parseApiError(response.status, payload);
  }
  return response.text();
}

export async function requestBytes(path: string): Promise<Uint8Array> {
  const response = await fetch(path, {
    method: "GET",
    credentials: "same-origin",
  });
  if (!response.ok) {
    const payload = await readResponseJson(response).catch(() => null);
    throw parseApiError(response.status, payload);
  }
  const buffer = await response.arrayBuffer();
  return new Uint8Array(buffer);
}
