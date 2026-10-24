# Monitoring

Grafana dashboards and alert rules for the server's Prometheus metrics, kept next to the code that
exports them: a change to a metric should change its dashboard and alerts in the same PR.

| File | What |
|---|---|
| `dashboards/content-check.json` | Content check: policy in use, outcomes, block and error rates, TypeSafe response time, blocks by question, tokens and cost; content held back for a recheck |
| `alerts/content-check.yaml` | Content check alerts: checks failing, blocking a lot, slow; content waiting long for its recheck, or published unchecked |

Nothing here is secret. Contact points such as Discord webhooks stay in Grafana.

## Loading into Grafana

Provisioning: mount the directories into the Grafana container and Grafana loads them on start.

```yaml
# Grafana service in docker compose
volumes:
  - ./monitoring/alerts:/etc/grafana/provisioning/alerting:ro
  - ./monitoring/dashboards:/var/lib/grafana/dashboards/hachimi:ro
  - ./grafana-dashboards.yaml:/etc/grafana/provisioning/dashboards/hachimi.yaml:ro
```

```yaml
# grafana-dashboards.yaml
apiVersion: 1
providers:
  - name: hachimi
    type: file
    options: { path: /var/lib/grafana/dashboards/hachimi }
```

Provisioned dashboards and alert rules are read-only in the UI. To change one, edit it in the UI,
export it, replace the file here and open a PR; or edit the file directly.

Without provisioning, import the dashboard JSON in the UI (Dashboards → New → Import) and create the
alert rules from the queries in the YAML.

## What the files assume

- **Dashboards** pick the Prometheus data source through the `datasource` variable, so they work with
  any data source UID. When exporting from the UI, keep panels on `${datasource}` rather than a fixed
  UID.
- **Alert rules** can't use variables: they query the data source with UID `prometheus`. Give the
  Prometheus data source that UID, or change `datasourceUid` in the YAML.
- Alert rules carry `service: hachimi-world-server` and `severity`. Route them to a contact point with
  a notification policy in Grafana.

## Adding a histogram

The exporter renders a histogram without configured buckets as a summary, which can't be combined
across the instances that run side by side during a rollout. Give every new histogram its buckets in
`src/web/web_metrics.rs` (`prometheus_builder`).

## Content check metrics

| Metric | Type | Labels |
|---|---|---|
| `content_check_total` | counter | `subject`, `outcome` (`pass`, `block`, `error`) |
| `content_check_blocked_total` | counter | `subject`, `question` |
| `content_check_duration_seconds` | histogram, 0.05–5 s | `subject`, `outcome` |
| `content_check_input_tokens_total` | counter | `subject` |
| `content_check_policy_info` | gauge, 1 while checks are on | `version`, `model` |
| `content_check_reused_total` | counter | `subject`, `outcome` (`pass`, `block`) |

`error` means TypeSafe failed or its answer didn't match the policy; the content was let through. A
check of content already judged under the same policy and model reuses that answer: it counts in
`content_check_reused_total` only, not in the others, which are about calls to TypeSafe. The
alerts need a minimum number of checks before they fire, because profile edits are few; with no
checks at all there's no data, which counts as OK.

### Rechecks

Content that must not pass unchecked, so far comments, is held back when its check fails, and a
background worker checks it again (`service::content_review`). After 24 hours it is published
unchecked.

| Metric | Type | Labels |
|---|---|---|
| `content_review_settled_total` | counter | `subject`, `outcome` (`pass`, `block`, `gave_up`, `unchecked`, `gone`) |
| `content_review_pending` | gauge | `subject` |
| `content_review_oldest_pending_seconds` | gauge | `subject` |

`gave_up` is content published unchecked after 24 hours, `unchecked` content published because checks
are off, and `gone` content deleted before it could be checked. The gauges are read from the database
every 30 seconds by every replica, so they are the same everywhere: read them with `max`. The
rechecks themselves are calls to TypeSafe and also count in the content check metrics.
