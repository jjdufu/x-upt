use std::sync::OnceLock;

use crate::service::ServiceError;
use crate::types::posts::{Post, PostSort, PostsRequest, PostsResponse};

fn catalog() -> &'static [Post] {
    static CATALOG: OnceLock<Vec<Post>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut frozen = vec![
            Post { user_id: 1, id: 1,  title: "sunt aut facere repellat provident occaecati excepturi optio reprehenderit".to_string(), body: "quia et suscipit\nsuscipit recusandae consequuntur expedita et cum\nreprehenderit molestiae ut ut quas totam\nnostrum rerum est autem sunt rem eveniet architecto".to_string() },
            Post { user_id: 1, id: 2,  title: "qui est esse".to_string(), body: "est rerum tempore vitae\nsequi sint nihil reprehenderit dolor beatae ea dolores neque\nfugiat blanditiis voluptate porro vel nihil molestiae ut reiciendis\nqui aperiam non debitis possimus qui neque nisi nulla".to_string() },
            Post { user_id: 1, id: 3,  title: "ea molestias quasi exercitationem repellat qui ipsa sit aut".to_string(), body: "et iusto sed quo iure\nvoluptatem occaecati omnis eligendi aut ad\nvoluptatem doloribus vel accusantium quis pariatur\nmolestiae porro eius odio et labore et velit aut".to_string() },
            Post { user_id: 1, id: 4,  title: "eum et est occaecati".to_string(), body: "ullam et saepe reiciendis voluptatem adipisci\nsit amet autem assumenda provident rerum culpa\nquis hic commodi nesciunt rem tenetur doloremque ipsam iure\nquis sunt voluptatem rerum illo velit".to_string() },
            Post { user_id: 1, id: 5,  title: "nesciunt quas odio".to_string(), body: "repudiandae veniam quaerat sunt sed\nalias aut fugiat sit autem sed est\nvoluptatem omnis possimus esse voluptatibus quis\nest aut tenetur dolor neque".to_string() },
            Post { user_id: 2, id: 6,  title: "Pricing API 接入方案设计与实现".to_string(), body: "设计并实现统一费率 Pricing API 的接入方案，支持多币种实时汇率转换与阶梯费率计算，需保证高可用与低延迟响应".to_string() },
            Post { user_id: 2, id: 7,  title: "用户鉴权中间件重构".to_string(), body: "将现有 RBAC 鉴权中间件重构为基于 JWT + OAuth2.0 的双模鉴权体系，支持服务间调用与用户态请求的统一拦截与放行".to_string() },
            Post { user_id: 2, id: 8,  title: "支付对账系统延迟优化".to_string(), body: "分析支付对账系统每日峰值期的高延迟问题，优化数据库查询索引与批处理策略，将 T+1 对账完成时间从 6 小时压缩至 30 分钟内".to_string() },
            Post { user_id: 3, id: 9,  title: "Gateway 熔断降级策略升级".to_string(), body: "基于 Sentinel 实现 API Gateway 的细粒度熔断降级策略，按接口维度配置 QPS 阈值与慢调用比例，保障核心交易链路稳定性".to_string() },
            Post { user_id: 3, id: 10, title: "分布式事务 Seata AT 模式迁移".to_string(), body: "将原有 TCC 模式的分布式事务逐步迁移至 Seata AT 模式，降低业务侵入性，同时保证最终一致性及全局事务的隔离级别".to_string() },
            Post { user_id: 3, id: 11, title: "全链路压测方案设计与执行".to_string(), body: "设计基于 ChaosBlade + JMeter 的全链路压测方案，覆盖网关、应用、中间件、数据库四层，输出各节点容量水位与瓶颈分析报告".to_string() },
            Post { user_id: 4, id: 12, title: "配置中心 Apollo 到 Nacos 迁移".to_string(), body: "制定 Apollo 到 Nacos 的平滑迁移方案，支持配置双向同步与灰度切换，确保存量应用无感迁移并兼容历史配置格式".to_string() },
            Post { user_id: 4, id: 13, title: "异步任务消息队列选型与实践".to_string(), body: "对比 RocketMQ、RabbitMQ、Pulsar 三种消息队列在异步任务场景下的吞吐、延迟与运维成本，选定 RocketMQ 并完成核心业务的消息异步化改造".to_string() },
            Post { user_id: 4, id: 14, title: "容器化部署 K8s 迁移方案".to_string(), body: "将传统 VM 部署的 20+ 微服务迁移至 K8s 集群，设计 Helm Chart 模板与 ArgoCD 持续交付流水线，实现灰度发布与自动扩缩容".to_string() },
            Post { user_id: 5, id: 15, title: "数据库分库分表 ShardingSphere 实践".to_string(), body: "针对订单表日均千万级数据量，基于 ShardingSphere 实施分库分表方案，按用户 ID 哈希分 16 库 64 表，查询延迟降低 90%".to_string() },
            Post { user_id: 5, id: 16, title: "日志采集系统 ELK 到 Loki 迁移".to_string(), body: "为降低日志存储成本，将 ELK 日志系统迁移至 Grafana Loki + MinIO 方案，存储成本降低 70%，查询性能提升 3 倍".to_string() },
            Post { user_id: 5, id: 17, title: "单元测试覆盖率提升专项".to_string(), body: "制定增量代码 90%、存量代码 60% 的覆盖率目标，引入 cargo-nextest + tarpaulin 工具链，建设 CI 门禁阻止覆盖率下滑".to_string() },
            Post { user_id: 6, id: 18, title: "gRPC 双向流推送服务设计".to_string(), body: "基于 gRPC 双向流模式设计实时行情推送服务，支持百万级并发连接与 10ms 以内消息投递延迟，实现客户端断线重连与消息补推".to_string() },
            Post { user_id: 6, id: 19, title: "OpenTelemetry 可观测性建设".to_string(), body: "基于 OpenTelemetry 统一 Trace/Metric/Log 三点数据采集，接入 Jaeger 分布式追踪与 Prometheus 监控告警，实现端到端可观测性覆盖".to_string() },
            Post { user_id: 6, id: 20, title: "Redis 缓存一致性治理".to_string(), body: "针对缓存与数据库一致性问题，设计基于 Canal + MQ 的最终一致性方案，配合本地缓存双删策略，将缓存不一致窗口缩短至 100ms 以内".to_string() },
        ];
        frozen.extend(super::posts_gen::generate());
        frozen
    })
}

pub fn search_posts(req: &PostsRequest) -> Result<PostsResponse, ServiceError> {
    if req.page < 1 {
        return Err(ServiceError::bad_request("page must be >= 1"));
    }
    if !(1..=20).contains(&req.page_size) {
        return Err(ServiceError::bad_request(
            "page_size must be between 1 and 20",
        ));
    }

    let mut hits: Vec<Post> = catalog()
        .iter()
        .filter(|post| {
            post.user_id == req.user_id
                && (post.title.contains(&req.keyword) || post.body.contains(&req.keyword))
        })
        .cloned()
        .collect();

    match req.sort {
        PostSort::IdAsc => hits.sort_by_key(|post| post.id),
        PostSort::IdDesc => hits.sort_by_key(|post| std::cmp::Reverse(post.id)),
        PostSort::TitleAsc => hits.sort_by(|a, b| a.title.cmp(&b.title)),
        PostSort::TitleDesc => hits.sort_by(|a, b| b.title.cmp(&a.title)),
    }

    let total = hits.len();
    let page = usize::try_from(req.page).unwrap_or(usize::MAX);
    let page_size = usize::try_from(req.page_size).unwrap_or(0);
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let items = hits.into_iter().skip(offset).take(page_size).collect();

    Ok(PostsResponse {
        items,
        total,
        page: req.page,
        page_size: req.page_size,
    })
}
