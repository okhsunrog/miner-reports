### Load Testing with Grafana k6

I've included some load tests using Grafana k6. You can get it here: https://k6.io/

The scripts are in the `load_tests/` folder.

**Standard test:**

```bash
k6 run load_tests/load-test.js
```

**High-load stress test:**

```bash
k6 run load_tests/high-load-test.js
```

### Running the Application

**First version, single actor**

```bash
cargo run --release --bin single_actor
```

**Second version, actor_per_pool**

```bash
cargo run --release --bin actor_per_pool
```

**Third version, using rayoun**

```bash
cargo run --release --bin rayon
```

---

### Benchmarking Results

This section summarizes the results of the `high-load-test.js` stress test across the three different architectural implementations.

#### Test Environment
*   **CPU:** 11th Gen Intel i5-1135G7 (8 Threads) @ 4.200GHz
*   **k6 Load Profile:**
    *   **Ramp up:** 0 to 500 Virtual Users over 1 minute.
    *   **Hold:** 500 Virtual Users for 2 minutes.
    *   **Ramp down:** 500 to 0 Virtual Users over 30 seconds.

#### Performance Comparison

| Metric | 1. Single Actor | 2. Actor per Pool | 3. Rayon (`SegQueue`) |
| :--- | :--- | :--- | :--- |
| **Throughput (RPS)** | ~12,100 | ~31,100 | **~33,900** |
| **Total Requests** | ~2.90 million | ~6.54 million | **~7.13 million** |
| **Avg. Latency** | ~25.2 ms | ~9.98 ms | **~6.62 ms** |
| **p(95) Latency** | **~12.1 ms** | ~15.9 ms | ~18.8 ms |
| **Error Rate** | 0.01% | 0.00% | **0.00%** |
| **Data Sent** | ~759 MB | ~1.7 GB | **~1.9 GB** |

