-- 账号模型预激活/预热：支持按每日指定时间对 OAuth 账号发起极微小请求，
-- 提前并行启动 5h 配额滑动窗口，最大化全天可用额度。默认关闭。
alter table runtime_settings
    add column account_warmup_enabled boolean not null default false,
    add column account_warmup_schedule_time text not null default '08:00'
        check (
            octet_length(account_warmup_schedule_time) between 5 and 255
            and account_warmup_schedule_time = btrim(account_warmup_schedule_time)
            and account_warmup_schedule_time !~ '[[:cntrl:]]'
            and account_warmup_schedule_time ~ '^([01][0-9]|2[0-3]):[0-5][0-9](,([01][0-9]|2[0-3]):[0-5][0-9])*$'
        ),
    add column account_warmup_model text
        check (
            account_warmup_model is null
            or (
                octet_length(account_warmup_model) between 1 and 128
                and account_warmup_model = btrim(account_warmup_model)
                and account_warmup_model !~ '[[:cntrl:]]'
            )
        );
