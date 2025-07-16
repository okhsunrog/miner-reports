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
