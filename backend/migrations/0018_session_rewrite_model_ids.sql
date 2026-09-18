-- 使用完整上游模型 ID；旧迁移保持冻结，通过增量迁移修正存量选择。
ALTER TABLE provider_accounts ALTER COLUMN session_keepalive_models
    SET DEFAULT ARRAY['gpt-5.6-sol', 'gpt-6-astra'];

-- 只替换旧默认名称，保留其他自定义模型；转换后去重并维持首次出现的顺序。
UPDATE provider_accounts AS account
SET session_keepalive_models = (
    SELECT array_agg(corrected.model ORDER BY corrected.first_position)
    FROM (
        SELECT CASE item.model
            WHEN '5.6 sol' THEN 'gpt-5.6-sol'
            WHEN '6' THEN 'gpt-6-astra'
            WHEN 'gpt-6' THEN 'gpt-6-astra'
            ELSE item.model
        END AS model, min(item.position) AS first_position
        FROM unnest(account.session_keepalive_models) WITH ORDINALITY AS item(model, position)
        GROUP BY 1
    ) AS corrected
)
WHERE session_keepalive_models && ARRAY['5.6 sol', '6', 'gpt-6'];
