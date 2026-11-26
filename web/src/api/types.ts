export type BusinessMethodId =
  | "x_upt.posts.search"
  | "x_upt.photos.render"
  | "x_upt.imgdef.compare";

export type ImageScene = "article_cover" | "avatar" | "banner";

export type FrontCategory = "core" | "envelope";

export type ContractName =
  | "grpc.proto"
  | "grpc.fds"
  | "soap.wsdl"
  | "graphql.sdl"
  | "upstream-openapi.json"
  | "admin-openapi.json";

export interface ApiErrorBody {
  error: string;
  message: string;
  missing?: string[];
}

export interface AuthSession {
  auth_required: boolean;
  authenticated: boolean;
}

export interface GlobalSettings {
  latency_min_ms: number;
  latency_max_ms: number;
  token_expire_minutes: number;
  token_cleanup_seconds: number;
  token_max_per_client: number;
  sign_window_seconds: number;
  sign_nonce_ttl_seconds: number;
  job_expire_seconds: number;
}

export interface MethodStatus {
  method: BusinessMethodId;
  label: string;
  enabled: boolean;
  can_enable: boolean;
  missing_preconditions: string[];
}

export interface FrontStatus {
  name: string;
  category: FrontCategory;
  protocol: string;
  enabled: boolean;
  running: boolean;
  host: string;
  port: number;
  http2: boolean;
  h2c: boolean;
  expire_minutes: number;
  auth: string[];
  supported_auth: string[];
  tls_enabled: boolean;
  has_tls_cert: boolean;
  has_tls_key: boolean;
  has_tls_client_ca: boolean;
  ws_max_conn: number;
  ws_ping_secs: number;
  job_workers: number;
  profile_count: number;
  can_enable: boolean;
  missing_preconditions: string[];
}

export interface SystemCounts {
  images: number;
  images_by_scene: {
    article_cover: number;
    avatar: number;
    banner: number;
  };
  posts: number;
  apps: number;
  envelope_secrets: number;
  envelope_profiles: number;
}

export interface SystemStatus {
  version: string;
  admin_listen: string;
  counts: SystemCounts;
  settings: GlobalSettings;
  methods: MethodStatus[];
  fronts: FrontStatus[];
}

export interface FrontUpdatePayload {
  host: string;
  port: number;
  http2: boolean;
  h2c: boolean;
  expire_minutes: number;
  auth: string[];
  tls_enabled: boolean;
  tls_cert_pem?: string | null;
  tls_key_pem?: string | null;
  tls_client_ca_pem?: string | null;
  ws_max_conn: number;
  ws_ping_secs: number;
  job_workers: number;
}

export interface ImageMeta {
  id: string;
  filename: string;
  scene: ImageScene;
  mime: string;
  width: number;
  height: number;
  size: number;
  ahash: string;
  created_at: string;
}

export interface PostRecord {
  id: number;
  userId: number;
  title: string;
  body: string;
}

export interface PostInput {
  id?: number;
  userId: number;
  title: string;
  body: string;
}

export interface PostsPage {
  items: PostRecord[];
  total: number;
  page: number;
  page_size: number;
}

export interface AppMeta {
  client_id: string;
  name: string;
  has_secret: boolean;
  secret_preview: string;
  has_rsa_public_pem: boolean;
  methods: string[];
  created_at: string;
}

export interface AppUpsertPayload {
  client_id: string;
  client_secret: string;
  name: string;
  rsa_public_pem?: string | null;
  methods: string[];
}

export interface SecretMeta {
  name: string;
  secret_kind: string;
  byte_len: number;
  fingerprint: string;
  public_material?: string | null;
  updated_at: string;
}

export interface SecretUpsertPayload {
  name: string;
  generate?: string;
  secret_kind?: string;
  value?: string;
}

export interface EnvelopeProfilesResponse {
  front: string;
  profiles: unknown[];
}

export interface EndpointDoc {
  method: string;
  path: string;
  description: string;
  request_example: string;
  response_example: string;
  curl_example: string;
}

export interface FrontDoc {
  name: string;
  category: FrontCategory;
  protocol: string;
  host: string;
  port: number;
  enabled: boolean;
  summary: string;
  auth_notes: string;
  endpoints: EndpointDoc[];
}

export interface AccessLogEntry {
  request_id: string;
  ts: string;
  level: string;
  event: string;
  method: string;
  path: string;
  status: number;
  code: unknown;
  duration: number;
  front: string;
  port: number;
  rpc?: string | null;
  auth?: string | null;
  error?: string | null;
  in: unknown;
  out: unknown;
}

export interface JobEntry {
  job_id: string;
  client_id: string;
  method: string;
  status: string;
  error?: string | null;
}
