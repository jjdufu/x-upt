import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { isAppMeta, isImageMeta, isPostsPage } from "../api/assets";
import { isFrontDoc } from "../api/docs";
import { isEnvelopeProfilesResponse, isSecretMeta } from "../api/envelopes";
import { isApiErrorBody, parseApiError } from "../api/http";
import { isAccessLogEntry, isJobEntry } from "../api/logs";
import {
  isAuthSession,
  isFrontStatus,
  isMethodStatus,
  isSystemStatus,
} from "../api/status";
import type { MethodStatus } from "../api/types";
import MethodSwitchCard from "../components/MethodSwitchCard.vue";
import UiSelect from "../components/ui/UiSelect.vue";

describe("API type guards", () => {
  it("validates structured error payloads and parses ApiRequestError", () => {
    const raw: unknown = {
      error: "precondition_failed",
      message: "Missing scene images",
      missing: ["article_cover", "avatar"],
    };
    expect(isApiErrorBody(raw)).toBe(true);
    const parsed = parseApiError(409, raw);
    expect(parsed.status).toBe(409);
    expect(parsed.code).toBe("precondition_failed");
    expect(parsed.missing).toEqual(["article_cover", "avatar"]);

    expect(isApiErrorBody({ error: 123 })).toBe(false);
    const fallback = parseApiError(500, "unexpected");
    expect(fallback.status).toBe(500);
    expect(fallback.code).toBe("http_500");
  });

  it("validates auth session, method status, front status, and system status", () => {
    expect(isAuthSession({ auth_required: false, authenticated: true })).toBe(true);
    expect(isAuthSession({ auth_required: "no", authenticated: true })).toBe(false);

    const methodObj = {
      method: "x_upt.photos.render",
      label: "图片渲染 (photos.render)",
      enabled: false,
      can_enable: false,
      missing_preconditions: ["需导入三类场景图片"],
    };
    const method: unknown = methodObj;
    expect(isMethodStatus(method)).toBe(true);
    expect(isMethodStatus({ ...methodObj, method: "unknown.method" })).toBe(false);

    const front: unknown = {
      name: "rest",
      category: "core",
      protocol: "REST HTTP/1.1",
      enabled: false,
      running: false,
      host: "0.0.0.0",
      port: 16002,
      http2: false,
      h2c: false,
      expire_minutes: 30,
      auth: ["api_key"],
      supported_auth: ["none", "api_key"],
      tls_enabled: false,
      has_tls_cert: false,
      has_tls_key: false,
      has_tls_client_ca: false,
      ws_max_conn: 1024,
      ws_ping_secs: 30,
      job_workers: 4,
      profile_count: 0,
      can_enable: false,
      missing_preconditions: ["需至少开启 1 个业务子接口"],
    };
    expect(isFrontStatus(front)).toBe(true);

    const statusPayload: unknown = {
      version: "0.1.0",
      admin_listen: "0.0.0.0:18768",
      counts: {
        images: 0,
        images_by_scene: { article_cover: 0, avatar: 0, banner: 0 },
        posts: 0,
        apps: 0,
        envelope_secrets: 0,
        envelope_profiles: 0,
      },
      settings: {
        latency_min_ms: 20,
        latency_max_ms: 100,
        token_expire_minutes: 30,
        token_cleanup_seconds: 300,
        token_max_per_client: 0,
        sign_window_seconds: 300,
        sign_nonce_ttl_seconds: 600,
        job_expire_seconds: 600,
      },
      methods: [method],
      fronts: [front],
    };
    expect(isSystemStatus(statusPayload)).toBe(true);
  });

  it("validates asset, envelope, doc, and log guards", () => {
    expect(
      isImageMeta({
        id: "img_1",
        filename: "cover.png",
        scene: "article_cover",
        mime: "image/png",
        width: 16,
        height: 16,
        size: 120,
        ahash: "00ff00ff00ff00ff",
        created_at: "2026-01-01T00:00:00Z",
      }),
    ).toBe(true);

    expect(
      isPostsPage({
        items: [{ id: 1, userId: 2, title: "Hello", body: "World" }],
        total: 1,
        page: 1,
        page_size: 50,
      }),
    ).toBe(true);

    expect(
      isAppMeta({
        client_id: "demo",
        name: "Demo",
        has_secret: true,
        secret_preview: "sec***",
        has_rsa_public_pem: false,
        methods: ["x_upt.posts.search"],
        created_at: "2026-01-01T00:00:00Z",
      }),
    ).toBe(true);

    expect(
      isSecretMeta({
        name: "aes.key",
        secret_kind: "bytes_hex",
        byte_len: 16,
        fingerprint: "abcd1234",
        public_material: null,
        updated_at: "2026-01-01T00:00:00Z",
      }),
    ).toBe(true);

    expect(
      isEnvelopeProfilesResponse({
        front: "envelope_token_plain",
        profiles: [],
      }),
    ).toBe(true);

    expect(
      isFrontDoc({
        name: "rest",
        category: "core",
        protocol: "REST HTTP/1.1",
        host: "0.0.0.0",
        port: 16002,
        enabled: false,
        summary: "REST Front",
        auth_notes: "api_key",
        endpoints: [
          {
            method: "POST",
            path: "/posts/search",
            description: "Search posts",
            request_example: "{}",
            response_example: "{}",
            curl_example: "curl ...",
          },
        ],
      }),
    ).toBe(true);

    expect(
      isAccessLogEntry({
        request_id: "req-1",
        ts: "2026-01-01T00:00:00Z",
        level: "INFO",
        event: "access",
        method: "POST",
        path: "/rpc",
        status: 200,
        code: 0,
        duration: 12,
        front: "jsonrpc",
        port: 16003,
        rpc: "x_upt.posts.search",
        auth: "api_key",
        error: null,
        in: {},
        out: {},
      }),
    ).toBe(true);

    expect(
      isJobEntry({
        job_id: "job-1",
        client_id: "demo",
        method: "x_upt.posts.search",
        status: "succeeded",
        error: null,
      }),
    ).toBe(true);
  });
});

describe("MethodSwitchCard & UiSelect", () => {
  it("disables switch and renders missing preconditions when can_enable is false", async () => {
    const lockedItem: MethodStatus = {
      method: "x_upt.photos.render",
      label: "图片渲染 (photos.render)",
      enabled: false,
      can_enable: false,
      missing_preconditions: ["缺少 article_cover 场景图片"],
    };

    const wrapper = mount(MethodSwitchCard, {
      props: { item: lockedItem },
    });

    const sw = wrapper.get('[data-testid="method-switch-x_upt.photos.render"]');
    expect(sw.attributes("disabled")).toBeDefined();

    const badge = wrapper.get('[data-testid="method-badge-x_upt.photos.render"]');
    expect(badge.text()).toBe("未就绪");

    const missing = wrapper.get('[data-testid="method-missing-x_upt.photos.render"]');
    expect(missing.text()).toContain("缺少 article_cover 场景图片");

    await sw.trigger("click");
    expect(wrapper.emitted("toggle")).toBeUndefined();

    const gotoBtn = wrapper.get('[data-testid="method-goto-import-x_upt.photos.render"]');
    await gotoBtn.trigger("click");
    expect(wrapper.emitted("gotoImport")?.length).toBe(1);
  });

  it("unlocks switch when can_enable becomes true and displays 运行中 when enabled", async () => {
    const readyItem: MethodStatus = {
      method: "x_upt.photos.render",
      label: "图片渲染 (photos.render)",
      enabled: false,
      can_enable: true,
      missing_preconditions: [],
    };

    const wrapper = mount(MethodSwitchCard, {
      props: { item: readyItem },
    });

    const sw = wrapper.get('[data-testid="method-switch-x_upt.photos.render"]');
    expect(sw.attributes("disabled")).toBeUndefined();
    expect(
      wrapper.get('[data-testid="method-badge-x_upt.photos.render"]').text(),
    ).toBe("已停用");

    await sw.trigger("click");
    expect(wrapper.emitted("toggle")?.[0]).toEqual([readyItem, true]);

    await wrapper.setProps({
      item: { ...readyItem, enabled: true },
    });
    expect(
      wrapper.get('[data-testid="method-badge-x_upt.photos.render"]').text(),
    ).toBe("运行中");
  });

  it("emits selected value from UiSelect", async () => {
    const wrapper = mount(UiSelect, {
      props: {
        modelValue: "article_cover",
        testId: "scene-select",
        options: [
          { value: "article_cover", label: "封面" },
          { value: "avatar", label: "头像" },
        ],
      },
    });

    const select = wrapper.get('[data-testid="scene-select"]');
    await select.setValue("avatar");
    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["avatar"]);
  });
});
