### Load Testing with Grafana k6

The repository includes three distinct load tests using [Grafana k6](https://k6.io/) to evaluate performance.

**Standard Test (`load-test.js`)**
A light smoke test (50 VUs) to verify functionality and responsiveness under normal load with strict latency checks.
```bash
k6 run load_tests/load-test.js
```

**High-Load Test (`high-load-test.js`)**
A stress test (500 VUs) using only 6 pools (low cardinality) to measure the raw throughput and processing power of the core architecture.
```bash
k6 run load_tests/high-load-test.js
```

**Wide-Load Test (`wide-load-test.js`)**
A scalability test (500 VUs) using 64 pools (high cardinality). This is designed to measure how effectively the system distributes a wide workload across a large number of CPU cores, particularly benefiting the `rayon` architecture.
```bash
k6 run load_tests/wide-load-test.js
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

**Third version, using rayon**
```bash
cargo run --release --bin rayon
```

---

### Benchmarking Results

This section summarizes the results of the high-load tests across the three different architectural implementations and three distinct hardware platforms.

#### Test Environments
*   **Laptop:** Intel Core i5-1135G7 (8 Threads) @ 4.200GHz
*   **Desktop PC:** AMD Ryzen 5 5600G (12 Threads) @ 4.464GHz
*   **Cloud VPS:** 32-Core Intel Xeon Platinum @ 2.70GHz

#### k6 Load Profile (for all tests)
*   **Ramp up:** 0 to 500 Virtual Users over 1 minute.
*   **Hold:** 500 Virtual Users for 2 minutes.
*   **Ramp down:** 500 to 0 Virtual Users over 30 seconds.

### Performance on Laptop (Intel i5, 8 Threads)
*Test: `high-load-test.js` (6 pools)*
| Metric | 1. Single Actor | 2. Actor per Pool | 3. Rayon (`SegQueue`) |
| :--- | :--- | :--- | :--- |
| **Throughput (RPS)** | ~12,100 | ~31,100 | **~33,900** |
| **Total Requests** | ~2.90 million | ~6.54 million | **~7.13 million** |
| **Avg. Latency** | ~25.2 ms | ~9.98 ms | **~6.62 ms** |
| **p(95) Latency** | **~12.1 ms** | ~15.9 ms | ~18.8 ms |
| **Error Rate** | 0.01% | 0.00% | **0.00%** |
| **Data Sent** | ~759 MB | ~1.7 GB | **~1.9 GB** |

### Performance on Desktop PC (AMD Ryzen 5, 12 Threads)
*Test: `high-load-test.js` (6 pools)*
| Metric | 1. Single Actor | 2. Actor per Pool | 3. Rayon (`SegQueue`) |
| :--- | :--- | :--- | :--- |
| **Throughput (RPS)** | ~12,100 | ~35,700 | **~47,200** |
| **Total Requests** | ~2.90 million | ~7.50 million | **~9.91 million** |
| **Avg. Latency** | ~27.1 ms | ~10.7 ms | **~5.07 ms** |
| **p(95) Latency** | **~10.0 ms** | ~10.9 ms | ~15.1 ms |
| **Error Rate** | 0.00% | 0.00% | **0.00%** |
| **Data Sent** | ~757 MB | ~2.0 GB | **~2.6 GB** |

### Performance on 32-Core Server (Cloud VPS)
*Test: `wide-load-test.js` (64 pools)*
| Metric | 1. Single Actor | 2. Actor per Pool | 3. Rayon (`SegQueue`) |
| :--- | :--- | :--- | :--- |
| **Throughput (RPS)** | ~15,100 | ~62,900 | **~76,800** |
| **Total Requests** | ~3.17 million | ~13.20 million | **~16.13 million** |
| **Avg. Latency** | ~25.3 ms | ~5.81 ms | **~2.28 ms** |
| **p(95) Latency**| **~4.73 ms** | ~8.97 ms | ~6.26 ms |
| **Error Rate** | 0.00% | 0.00% | **0.00%** |
| **Data Sent** | ~835 MB | ~3.5 GB | **~4.3 GB** |