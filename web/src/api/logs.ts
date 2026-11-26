import {
  isArrayOf,
  isNumber,
  isOptionalNullableString,
  isRecord,
  isString,
  requestJson,
} from "./http";
import type { AccessLogEntry, JobEntry } from "./types";

export function isAccessLogEntry(value: unknown): value is AccessLogEntry {
  return (
    isRecord(value) &&
    isString(value.request_id) &&
    isString(value.ts) &&
    isString(value.level) &&
    isString(value.event) &&
    isString(value.method) &&
    isString(value.path) &&
    isNumber(value.status) &&
    isNumber(value.duration) &&
    isString(value.front) &&
    isNumber(value.port) &&
    isOptionalNullableString(value.rpc) &&
    isOptionalNullableString(value.auth) &&
    isOptionalNullableString(value.error)
  );
}

function isRecentLogsResponse(
  value: unknown,
): value is { items: AccessLogEntry[] } {
  return isRecord(value) && isArrayOf(value.items, isAccessLogEntry);
}

export function isJobEntry(value: unknown): value is JobEntry {
  return (
    isRecord(value) &&
    isString(value.job_id) &&
    isString(value.client_id) &&
    isString(value.method) &&
    isString(value.status) &&
    isOptionalNullableString(value.error)
  );
}

function isJobsResponse(value: unknown): value is { items: JobEntry[] } {
  return isRecord(value) && isArrayOf(value.items, isJobEntry);
}

export interface LogQueryFilter {
  front?: string;
  status?: string;
  request_id?: string;
  limit?: number;
}

export function fetchRecentLogs(
  filter: LogQueryFilter = {},
): Promise<{ items: AccessLogEntry[] }> {
  const params = new URLSearchParams();
  if (filter.front && filter.front.trim().length > 0) {
    params.set("front", filter.front.trim());
  }
  if (filter.status && filter.status.trim().length > 0) {
    params.set("status", filter.status.trim());
  }
  if (filter.request_id && filter.request_id.trim().length > 0) {
    params.set("request_id", filter.request_id.trim());
  }
  params.set("limit", String(filter.limit ?? 100));
  return requestJson(`/api/v1/logs/recent?${params.toString()}`, isRecentLogsResponse);
}

export function fetchJobs(): Promise<{ items: JobEntry[] }> {
  return requestJson("/api/v1/jobs", isJobsResponse);
}
