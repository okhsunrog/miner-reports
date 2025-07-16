import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';

// --- Configuration for the Test ---

// Custom trends to track durations for each endpoint separately
const reportDuration = new Trend('report_req_duration');
const statsDuration = new Trend('stats_req_duration');

export const options = {
  // Define stages for the load test: ramp-up, hold, ramp-down
  stages: [
    { duration: '30s', target: 50 },  // Ramp up to 50 virtual users over 30 seconds
    { duration: '1m', target: 50 },   // Hold 50 virtual users for 1 minute
    { duration: '10s', target: 0 },   // Ramp down to 0 users
  ],

  // Define performance thresholds. The test will fail if these are not met.
  thresholds: {
    'http_req_failed': ['rate<0.01'], // http errors should be less than 1%
    'http_req_duration': ['p(95)<150'], // 95% of all requests must complete below 150ms
    'report_req_duration': ['p(95)<50'], // 95% of POST /report requests should be below 50ms
    'stats_req_duration': ['p(95)<10'], // 95% of GET /stats requests should be below 10ms (should be very fast!)
  },
};

// --- Test Logic ---

// Data for generating reports
const POOLS = ['us-east', 'eu-west', 'ap-south', 'cn-north'];
const BASE_URL = 'http://127.0.0.1:3000';

export default function () {
  // Use a random number to decide which action to perform.
  // This simulates many more reports than stats requests.
  if (Math.random() < 0.95) {
    // --- Action 1: Post a new report (95% of the time) ---

    // Generate dynamic report data
    const pool = POOLS[Math.floor(Math.random() * POOLS.length)];
    const workerId = `worker-${__VU}-${__ITER}`; // Unique worker ID per VU and iteration
    const hashrate = 30 + Math.random() * 30; // Random hashrate between 30 and 60
    const temperature = 65 + Math.random() * 25; // Random temp between 65 and 90

    // The timestamp must be in seconds, Date.now() is in milliseconds
    const timestamp = Math.floor(Date.now() / 1000);

    const payload = JSON.stringify({
      worker_id: workerId,
      pool: pool,
      hashrate: hashrate,
      temperature: temperature,
      timestamp: timestamp,
    });

    const params = {
      headers: {
        'Content-Type': 'application/json',
      },
    };

    const res = http.post(`${BASE_URL}/report`, payload, params);

    // Check if the request was successful and add duration to custom trend
    check(res, { 'POST /report status was 200': (r) => r.status === 200 });
    reportDuration.add(res.timings.duration);

  } else {
    // --- Action 2: Get stats (5% of the time) ---
    const res = http.get(`${BASE_URL}/stats`);

    // Check if the request was successful and add duration to custom trend
    check(res, {
      'GET /stats status was 200': (r) => r.status === 200,
      'GET /stats response is valid JSON': (r) => {
        try {
          // Ensure the body is valid JSON and has a 'pools' key
          return r.json('pools') !== null;
        } catch (e) {
          return false;
        }
      },
    });
    statsDuration.add(res.timings.duration);
  }

  // Add a short sleep to simulate a realistic interval between actions
  sleep(Math.random() * 2 + 1); // Sleep for 1-3 seconds
}
