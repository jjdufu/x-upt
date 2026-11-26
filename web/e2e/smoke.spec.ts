import { deflateSync } from "node:zlib";

import { expect, test } from "@playwright/test";

function crc32(buf: Uint8Array): number {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i += 1) {
    crc ^= buf[i] ?? 0;
    for (let bit = 0; bit < 8; bit += 1) {
      const mask = -(crc & 1);
      crc = (crc >>> 1) ^ (0xedb88320 & mask);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(typeAscii: string, data: Uint8Array): Buffer {
  const typeBytes = Buffer.from(typeAscii, "ascii");
  const lenBuf = Buffer.alloc(4);
  lenBuf.writeUInt32BE(data.length, 0);

  const crcInput = Buffer.concat([typeBytes, Buffer.from(data)]);
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(crcInput), 0);

  return Buffer.concat([lenBuf, typeBytes, Buffer.from(data), crcBuf]);
}

function createPng16x16(seed: number): Buffer {
  const width = 16;
  const height = 16;
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type: RGB
  ihdr[10] = 0; // compression
  ihdr[11] = 0; // filter
  ihdr[12] = 0; // interlace

  const stride = 1 + width * 3;
  const raw = Buffer.alloc(height * stride);
  for (let y = 0; y < height; y += 1) {
    const rowStart = y * stride;
    raw[rowStart] = 0; // filter: None
    for (let x = 0; x < width; x += 1) {
      const px = rowStart + 1 + x * 3;
      raw[px] = (x * 15 + seed * 41) & 0xff;
      raw[px + 1] = (y * 15 + seed * 73) & 0xff;
      raw[px + 2] = ((x ^ y) * 16 + seed * 19) & 0xff;
    }
  }

  const compressed = deflateSync(raw);
  return Buffer.concat([
    signature,
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", compressed),
    pngChunk("IEND", new Uint8Array(0)),
  ]);
}

test("blank precondition guard -> upload scene images -> unlock switches -> docs & contracts", async ({
  page,
  request,
}) => {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];

  page.on("console", (msg) => {
    if (msg.type() === "error") {
      consoleErrors.push(msg.text());
    }
  });
  page.on("pageerror", (err) => {
    pageErrors.push(err.message);
  });

  await page.goto("/");

  const photosSwitch = page.getByTestId("method-switch-x_upt.photos.render");
  const imgdefSwitch = page.getByTestId("method-switch-x_upt.imgdef.compare");

  await expect(photosSwitch).toBeVisible();
  await expect(imgdefSwitch).toBeVisible();
  expect(await photosSwitch.isDisabled()).toBe(true);
  expect(await imgdefSwitch.isDisabled()).toBe(true);
  await expect(page.getByTestId("method-badge-x_upt.photos.render")).toHaveText("未就绪");
  await expect(page.getByTestId("method-badge-x_upt.imgdef.compare")).toHaveText("未就绪");

  await page.getByTestId("nav-assets").click();
  await expect(page.getByTestId("image-scene-select")).toBeVisible();

  const scenes: ReadonlyArray<{ scene: "article_cover" | "avatar" | "banner"; seed: number }> = [
    { scene: "article_cover", seed: 1 },
    { scene: "avatar", seed: 2 },
    { scene: "banner", seed: 3 },
  ];

  for (let i = 0; i < scenes.length; i += 1) {
    const item = scenes[i]!;
    await page.getByTestId("image-scene-select").selectOption(item.scene);
    await page.getByTestId("image-file-input").setInputFiles({
      name: `${item.scene}.png`,
      mimeType: "image/png",
      buffer: createPng16x16(item.seed),
    });
    const uploadBtn = page.getByTestId("image-upload-btn");
    await expect(uploadBtn).toBeEnabled();
    await uploadBtn.click();
    await expect(uploadBtn).toBeDisabled();
    await expect(page.locator('[data-testid^="image-item-"]')).toHaveCount(i + 1);
  }

  await page.getByTestId("nav-fronts").click();
  await expect(photosSwitch).toBeEnabled();
  await expect(imgdefSwitch).toBeEnabled();
  expect(await photosSwitch.isDisabled()).toBe(false);
  expect(await imgdefSwitch.isDisabled()).toBe(false);

  await imgdefSwitch.click();
  await expect(page.getByTestId("method-badge-x_upt.imgdef.compare")).toHaveText("运行中");

  await photosSwitch.click();
  await expect(page.getByTestId("method-badge-x_upt.photos.render")).toHaveText("运行中");

  const restSwitch = page.getByTestId("front-switch-rest");
  expect(await restSwitch.isDisabled()).toBe(true);

  const e2eRestPort = Number(process.env.X_UPT_E2E_REST_PORT ?? "16002");
  const baseUrl = new URL(page.url());
  const csrfHeaders = {
    Origin: baseUrl.origin,
    Host: baseUrl.host,
  };
  const appRes = await request.post("/api/v1/apps", {
    headers: csrfHeaders,
    data: {
      client_id: "e2e_client",
      client_secret: "e2e_secret_12345",
      name: "E2E Client",
      methods: ["x_upt.photos.render", "x_upt.imgdef.compare"],
    },
  });
  expect(appRes.status()).toBe(200);

  const frontCfgRes = await request.put("/api/v1/fronts/rest", {
    headers: csrfHeaders,
    data: {
      host: "127.0.0.1",
      port: e2eRestPort,
      http2: false,
      h2c: false,
      expire_minutes: 30,
      auth: ["api_key"],
      tls_enabled: false,
      ws_max_conn: 1024,
      ws_ping_secs: 30,
      job_workers: 4,
    },
  });
  expect(frontCfgRes.status()).toBe(200);

  await page.getByTestId("refresh-status").click();
  await expect(restSwitch).toBeEnabled();
  await restSwitch.click();
  await expect(page.getByTestId("front-badge-rest")).toHaveText("运行中");

  await page.getByTestId("nav-docs").click();
  await expect(page.getByTestId("docs-detail-pane")).toBeVisible();

  await page.getByTestId("nav-contracts").click();
  const contractPreview = page.getByTestId("contract-preview");
  await expect(contractPreview).toBeVisible();
  await expect(contractPreview).toContainText("syntax");

  const fdsResponse = await request.get("/api/v1/contracts/grpc.fds?download=1");
  expect(fdsResponse.status()).toBe(200);
  const fdsBytes = await fdsResponse.body();
  expect(fdsBytes.byteLength).toBeGreaterThan(0);

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByTestId("contract-download-grpc.fds").click(),
  ]);
  expect(download.suggestedFilename()).toBeTruthy();

  expect(consoleErrors).toEqual([]);
  expect(pageErrors).toEqual([]);
});
