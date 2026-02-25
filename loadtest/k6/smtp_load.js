/**
 * K6 Load Test for SMTP
 *
 * This script provides an alternative to rusmes-loadtest using k6.
 *
 * Usage:
 *   k6 run smtp_load.js
 *   k6 run --vus 100 --duration 60s smtp_load.js
 */

import { check } from 'k6';
import { Trend, Counter } from 'k6/metrics';
import tcp from 'k6/x/tcp';

// Custom metrics
const smtpLatency = new Trend('smtp_latency');
const smtpErrors = new Counter('smtp_errors');
const smtpSuccess = new Counter('smtp_success');

// Test configuration
export const options = {
  stages: [
    { duration: '30s', target: 100 },   // Ramp-up to 100 users
    { duration: '60s', target: 100 },   // Stay at 100 users
    { duration: '30s', target: 0 },     // Ramp-down to 0 users
  ],
  thresholds: {
    'smtp_latency': ['p(95)<500'],      // 95% of requests < 500ms
    'smtp_errors': ['count<100'],        // Less than 100 errors
  },
};

// SMTP configuration
const SMTP_HOST = __ENV.SMTP_HOST || 'localhost';
const SMTP_PORT = __ENV.SMTP_PORT || '25';
const SMTP_FROM = __ENV.SMTP_FROM || 'loadtest@example.com';
const SMTP_TO = __ENV.SMTP_TO || 'recipient@example.com';

/**
 * Generate random email message
 */
function generateMessage(size) {
  const words = 'Lorem ipsum dolor sit amet consectetur adipiscing elit'.split(' ');
  let body = '';

  while (body.length < size) {
    body += words[Math.floor(Math.random() * words.length)] + ' ';
  }

  return `From: ${SMTP_FROM}\r\n` +
         `To: ${SMTP_TO}\r\n` +
         `Subject: K6 Load Test ${Date.now()}\r\n` +
         `\r\n` +
         `${body}\r\n`;
}

/**
 * Send SMTP message
 */
function sendSMTP() {
  const startTime = Date.now();

  try {
    // Connect to SMTP server
    const conn = tcp.connect(SMTP_HOST, SMTP_PORT);

    // Read greeting
    const greeting = conn.read();
    if (!greeting.includes('220')) {
      throw new Error('Invalid SMTP greeting: ' + greeting);
    }

    // EHLO
    conn.write('EHLO loadtest.k6.io\r\n');
    const ehlo = conn.read();
    if (!ehlo.includes('250')) {
      throw new Error('EHLO failed: ' + ehlo);
    }

    // MAIL FROM
    conn.write(`MAIL FROM:<${SMTP_FROM}>\r\n`);
    const mailFrom = conn.read();
    if (!mailFrom.includes('250')) {
      throw new Error('MAIL FROM failed: ' + mailFrom);
    }

    // RCPT TO
    conn.write(`RCPT TO:<${SMTP_TO}>\r\n`);
    const rcptTo = conn.read();
    if (!rcptTo.includes('250')) {
      throw new Error('RCPT TO failed: ' + rcptTo);
    }

    // DATA
    conn.write('DATA\r\n');
    const dataResp = conn.read();
    if (!dataResp.includes('354')) {
      throw new Error('DATA failed: ' + dataResp);
    }

    // Send message
    const message = generateMessage(1024 + Math.floor(Math.random() * 10240));
    conn.write(message);
    conn.write('\r\n.\r\n');
    const sendResp = conn.read();
    if (!sendResp.includes('250')) {
      throw new Error('Message send failed: ' + sendResp);
    }

    // QUIT
    conn.write('QUIT\r\n');
    conn.close();

    const latency = Date.now() - startTime;
    smtpLatency.add(latency);
    smtpSuccess.add(1);

    return true;
  } catch (error) {
    console.error('SMTP error:', error);
    smtpErrors.add(1);
    return false;
  }
}

/**
 * Main VU (Virtual User) function
 */
export default function() {
  sendSMTP();

  // Random think time between 0.1 and 1 second
  const thinkTime = 0.1 + Math.random() * 0.9;
  sleep(thinkTime);
}

/**
 * Setup function (runs once before test)
 */
export function setup() {
  console.log('Starting SMTP load test...');
  console.log(`Target: ${SMTP_HOST}:${SMTP_PORT}`);
  console.log(`From: ${SMTP_FROM}`);
  console.log(`To: ${SMTP_TO}`);
}

/**
 * Teardown function (runs once after test)
 */
export function teardown(data) {
  console.log('SMTP load test complete');
}
