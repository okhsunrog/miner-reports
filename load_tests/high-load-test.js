import http from 'k6/http';
import { check } from 'k6';
import { Trend } from 'k6/metrics';

// --- Configuration for the High-Load Test ---

const reportDuration = new Trend('report_req_duration');
const statsDuration = new Trend('stats_req_duration');

export const options = {
  // Stages for a much more aggressive load test
  stages: [
    { duration: '1m', target: 500 },   // Ramp up to 500 virtual users over 1 minute
    { duration: '2m', target: 500 },   // Hold 500 virtual users for 2 minutes
    { duration: '30s', target: 0 },    // Ramp down
  ],

  // More realistic thresholds for a high-load scenario.
  // The service is under extreme pressure, so latencies will be higher.
  thresholds: {
    'http_req_failed': ['rate<0.01'],      // Still expect <1% errors
    'http_req_duration': ['p(95)<250'],    // 95% of all requests must complete below 250ms
    'report_req_duration': ['p(95)<50'],   // POSTs will do more work, so give them a 50ms budget
    'stats_req_duration': ['p(95)<20'],    // GETs should still be very fast, budget 20ms
  },
};

// --- Test Logic ---

const POOLS = ['us-east', 'eu-west', 'ap-south', 'cn-north', 'sa-east', 'af-south'];
const BASE_URL = 'http://127.0.0.1:3000';

export default function () {
  // 95% of VUs send reports, 5% request stats
  if (Math.random() < 0.95) {
    // --- Action 1: Post a new report ---
    const pool = POOLS[Math.floor(Math.random() * POOLS.length)];
    const workerId = `worker-highload-${__VU}-${__ITER}`;
    const hashrate = 30 + Math.random() * 30;
    const temperature = 65 + Math.random() * 25;
    const timestamp = Math.floor(Date.now() / 1000);

    const payload = JSON.stringify({
      worker_id: workerId,
      pool: pool,
      hashrate: hashrate,
      temperature: temperature,
      timestamp: timestamp,
    });

    const params = {
      headers: { 'Content-Type': 'application/json' },
    };

    const res = http.post(`${BASE_URL}/report`, payload, params);
    check(res, { 'POST /report status was 200': (r) => r.status === 200 });
    reportDuration.add(res.timings.duration);

  } else {
    // --- Action 2: Get stats ---
    const res = http.get(`${BASE_URL}/stats`);
    check(res, { 'GET /stats status was 200': (r) => r.status === 200 });
    statsDuration.add(res.timings.duration);
  }

}
