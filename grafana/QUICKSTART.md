# Quick Start Guide - RusMES Grafana Dashboard

Get your RusMES monitoring dashboard up and running in 5 minutes.

## Prerequisites

- Grafana 10.x or later installed
- Prometheus installed and running
- RusMES server running with metrics enabled

## 5-Minute Setup

### Step 1: Configure RusMES Metrics (30 seconds)

Ensure your `rusmes.toml` has metrics enabled:

```toml
[metrics]
enabled = true
bind_address = "0.0.0.0:9090"
path = "/metrics"
```

Restart RusMES:

```bash
systemctl restart rusmes
```

Verify metrics endpoint:

```bash
curl http://localhost:9090/metrics | head -20
```

### Step 2: Configure Prometheus (1 minute)

Add RusMES to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'rusmes'
    static_configs:
      - targets: ['localhost:9090']
    scrape_interval: 30s
```

Reload Prometheus:

```bash
curl -X POST http://localhost:9090/-/reload
# OR
systemctl restart prometheus
```

Verify scraping:

```bash
curl http://localhost:9090/api/v1/targets | grep rusmes
```

### Step 3: Import Dashboard to Grafana (2 minutes)

#### Via Web UI:

1. Open Grafana: `http://localhost:3000`
2. Login (default: admin/admin)
3. Click **+** → **Import dashboard**
4. Click **Upload JSON file**
5. Select `grafana/dashboard.json`
6. Select **Prometheus** as data source
7. Click **Import**

#### Via Command Line:

```bash
# Copy dashboard file
sudo cp grafana/dashboard.json /var/lib/grafana/dashboards/

# Or use Grafana API
curl -X POST http://admin:admin@localhost:3000/api/dashboards/db \
  -H "Content-Type: application/json" \
  -d @grafana/dashboard.json
```

### Step 4: Load Alert Rules (1 minute)

```bash
# Copy alerts to Prometheus
sudo cp grafana/alerts.yaml /etc/prometheus/rules/

# Update prometheus.yml to include rules
echo "rule_files:
  - 'rules/alerts.yaml'" | sudo tee -a /etc/prometheus/prometheus.yml

# Reload Prometheus
curl -X POST http://localhost:9090/-/reload
```

Verify alerts loaded:

```bash
curl http://localhost:9090/api/v1/rules | grep rusmes
```

### Step 5: View Dashboard (30 seconds)

1. In Grafana, go to **Dashboards** → **Browse**
2. Click **RusMES Mail Server Monitoring**
3. Adjust time range if needed (top-right corner)
4. Done!

## Docker Quick Start

If you're using Docker Compose:

```yaml
version: '3.8'

services:
  rusmes:
    image: rusmes:latest
    ports:
      - "25:25"
      - "143:143"
      - "9090:9090"
    volumes:
      - ./rusmes.toml:/etc/rusmes/rusmes.toml

  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - ./grafana/alerts.yaml:/etc/prometheus/rules/alerts.yaml
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--web.enable-lifecycle'

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    volumes:
      - ./grafana/dashboard.json:/etc/grafana/provisioning/dashboards/rusmes.json
      - ./grafana-provisioning.yaml:/etc/grafana/provisioning/dashboards/default.yaml
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
      - GF_USERS_ALLOW_SIGN_UP=false
```

Create `grafana-provisioning.yaml`:

```yaml
apiVersion: 1
providers:
  - name: 'RusMES'
    orgId: 1
    folder: ''
    type: file
    disableDeletion: false
    updateIntervalSeconds: 10
    allowUiUpdates: true
    options:
      path: /etc/grafana/provisioning/dashboards
```

Start everything:

```bash
docker-compose up -d
```

Access Grafana at `http://localhost:3000` (admin/admin).

## Kubernetes Quick Start

```bash
# Create namespace
kubectl create namespace monitoring

# Install Prometheus (using Helm)
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm install prometheus prometheus-community/prometheus \
  --namespace monitoring \
  --set alertmanager.enabled=true \
  --set server.extraConfigmapMounts[0].name=rusmes-alerts \
  --set server.extraConfigmapMounts[0].mountPath=/etc/prometheus/rules \
  --set server.extraConfigmapMounts[0].configMap=rusmes-alerts \
  --set server.extraConfigmapMounts[0].readOnly=true

# Create alerts ConfigMap
kubectl create configmap rusmes-alerts \
  --from-file=alerts.yaml=grafana/alerts.yaml \
  --namespace monitoring

# Install Grafana (using Helm)
helm repo add grafana https://grafana.github.io/helm-charts
helm install grafana grafana/grafana \
  --namespace monitoring \
  --set adminPassword=admin

# Create dashboard ConfigMap
kubectl create configmap rusmes-dashboard \
  --from-file=dashboard.json=grafana/dashboard.json \
  --namespace monitoring

# Get Grafana admin password
kubectl get secret --namespace monitoring grafana -o jsonpath="{.data.admin-password}" | base64 --decode

# Port-forward to access Grafana
kubectl port-forward --namespace monitoring svc/grafana 3000:80
```

Then import the dashboard via the Grafana UI at `http://localhost:3000`.

## Verification Checklist

After setup, verify everything is working:

- [ ] Prometheus scraping RusMES metrics
  ```bash
  curl http://localhost:9090/api/v1/targets | grep rusmes
  ```

- [ ] Metrics returning data
  ```bash
  curl http://localhost:9090/api/v1/query?query=rusmes_mail_processed_total
  ```

- [ ] Alert rules loaded
  ```bash
  curl http://localhost:9090/api/v1/rules | grep -c rusmes
  # Should return ~20
  ```

- [ ] Dashboard showing data in Grafana
  - Navigate to dashboard
  - Check panels are populated
  - Verify no "No data" messages

- [ ] Alerts evaluated
  ```bash
  curl http://localhost:9090/api/v1/alerts | grep rusmes
  ```

## Common Issues

### No Data in Dashboard

**Problem**: Panels show "No data"

**Solutions**:
1. Check time range (top-right) - ensure it covers recent data
2. Verify Prometheus is scraping:
   ```bash
   curl http://localhost:9090/api/v1/targets
   ```
3. Check RusMES metrics endpoint:
   ```bash
   curl http://localhost:9090/metrics
   ```
4. Verify Grafana data source connection:
   - Settings → Data Sources → Prometheus → Test

### Alerts Not Loading

**Problem**: No alerts in Prometheus

**Solutions**:
1. Check Prometheus config includes rule_files:
   ```bash
   grep -A2 "rule_files:" /etc/prometheus/prometheus.yml
   ```
2. Verify alert file path is correct
3. Check Prometheus logs:
   ```bash
   journalctl -u prometheus -f
   ```
4. Reload Prometheus:
   ```bash
   curl -X POST http://localhost:9090/-/reload
   ```

### High CPU in Grafana

**Problem**: Grafana using excessive CPU

**Solutions**:
1. Increase auto-refresh interval (30s → 1m)
2. Reduce dashboard time range
3. Use recording rules in Prometheus for complex queries
4. Disable auto-refresh when not actively monitoring

## Next Steps

- Configure alert notifications: See [README.md](README.md#notification-channels)
- Customize thresholds: See [README.md](README.md#customization-guide)
- Set up recording rules: See [README.md](README.md#recording-rules)
- Enable HTTPS: See Grafana documentation
- Set up backups: Export dashboard JSON regularly

## Support

For detailed documentation, see:
- [README.md](README.md) - Full documentation
- [Metrics Reference](../crates/rusmes-metrics/README.md) - Metric details
- Main project docs: [../README.md](../README.md)

For issues:
- GitHub Issues: https://github.com/yourusername/rusmes/issues
- Check Prometheus: `http://localhost:9090`
- Check Grafana logs: `journalctl -u grafana -f`
