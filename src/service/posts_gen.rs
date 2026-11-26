use crate::types::posts::Post;

const FIRST_ID: u64 = 21;
const LAST_ID: u64 = 1020;

const SYSTEMS: &[&str] = &[
    "Kafka",
    "Redis",
    "MySQL",
    "Nginx",
    "Kubernetes",
    "Nacos",
    "Pulsar",
    "ClickHouse",
    "MinIO",
    "PostgreSQL",
    "RabbitMQ",
    "Elasticsearch",
];

const TOPICS: &[&str] = &[
    "鉴权", "对账", "缓存", "队列", "配置", "监控", "发布", "扩容", "限流", "审计", "调度", "存储",
];

const VERBS: &[&str] = &[
    "优化", "迁移", "重构", "治理", "接入", "排查", "降级", "加固",
];

const SHARED_TITLES: &[&str] = &[
    "重复告警收敛方案",
    "夜间批次失败复盘",
    "配置热更新回滚演练",
    "慢查询治理周报",
    "证书到期巡检",
];

const ERRORS: &[&str] = &[
    "ECONNRESET",
    "SQLSTATE 40001",
    "502 Bad Gateway",
    "429 Too Many Requests",
    "context deadline exceeded",
];

fn mix(id: u64, salt: u64) -> u64 {
    id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .rotate_left(13)
}

fn pick<'a>(id: u64, salt: u64, items: &'a [&str]) -> &'a str {
    items[(mix(id, salt) as usize) % items.len()]
}

fn user_for_id(id: u64) -> u64 {
    let n = id - FIRST_ID;
    match n {
        0..=71 => 10,
        72..=143 => 20,
        144..=173 => 6,
        174 => 7,
        175 => 13,
        176 => 37,
        _ => {
            const REST: [u64; 34] = [
                1, 2, 3, 4, 5, 8, 9, 11, 12, 14, 15, 16, 17, 18, 19, 21, 22, 23, 24, 25, 26, 27,
                28, 29, 30, 31, 32, 33, 34, 35, 36, 38, 39, 40,
            ];
            REST[((n - 177) as usize) % REST.len()]
        }
    }
}

fn tag_for(id: u64, user_id: u64) -> &'static str {
    let tag = match mix(id, 9) % 20 {
        0..=2 => "API",
        3..=4 => "网关",
        5 => "超时",
        6 => "timeout",
        7 => "retry",
        8 => "gRPC",
        _ => "",
    };
    if user_id == 6 && tag.contains("gRPC") {
        "protobuf"
    } else {
        tag
    }
}

fn title_for(id: u64, user_id: u64, tag: &str) -> String {
    if mix(id, 1).is_multiple_of(16) {
        return pick(id, 2, SHARED_TITLES).to_string();
    }
    let sys = pick(id, 3, SYSTEMS);
    let topic = pick(id, 4, TOPICS);
    let verb = pick(id, 5, VERBS);
    let mut title = if mix(id, 11).is_multiple_of(5) {
        format!("{sys} {topic} {verb} playbook #{id}")
    } else {
        format!("{sys}{topic}{verb}专项")
    };
    if !tag.is_empty() && mix(id, 12).is_multiple_of(2) {
        title.push(' ');
        title.push_str(tag);
    }
    if user_id == 6 && title.contains("gRPC") {
        title = title.replace("gRPC", "HTTP2");
    }
    title
}

fn body_for(id: u64, user_id: u64, tag: &str, title: &str) -> String {
    let sys = pick(id, 3, SYSTEMS);
    let err = pick(id, 6, ERRORS);
    let ticket = format!("INC-{id:04}");
    let trace = format!("trc-{:016x}", mix(id, 7));
    let mut body =
        format!("工单 {ticket}：{title}\n涉及 {sys}，最近一次错误 {err}，trace={trace}。");
    if !tag.is_empty() {
        body.push_str(&format!("\n关键词 {tag} 出现在变更窗口与回滚检查清单中。"));
    }
    if mix(id, 8).is_multiple_of(7) {
        let para = format!(
            "\n复盘记录 {ticket}：变更单已合并到生产，观察期 30 分钟。\n\
             1) 核对 {sys} 连接池与超时；2) 对照 {err} 是否复现；3) 保留 {trace} 供审计。\n\
             附：灰度 5% → 20% → 100%，每档核对错误率与 P99。下游调用方需准备 retry 与熔断阈值。"
        );
        body.push_str(&para.repeat(6));
    }
    if user_id == 6 {
        body = body.replace("gRPC", "HTTP2");
    }
    body
}

fn one(id: u64) -> Post {
    let user_id = user_for_id(id);
    let tag = tag_for(id, user_id);
    let title = title_for(id, user_id, tag);
    let body = body_for(id, user_id, tag, &title);
    debug_assert!(
        user_id != 6 || (!title.contains("gRPC") && !body.contains("gRPC")),
        "user 6 must not contain gRPC"
    );
    Post {
        user_id,
        id,
        title,
        body,
    }
}

#[must_use]
pub fn generate() -> Vec<Post> {
    (FIRST_ID..=LAST_ID).map(one).collect()
}
