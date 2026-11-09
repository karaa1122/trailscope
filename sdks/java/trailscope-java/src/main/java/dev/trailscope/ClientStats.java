package dev.trailscope;

/**
 * Counters of a client.
 *
 * @param queued items waiting to be sent
 * @param dropped items dropped because the queue was full
 * @param sent items the server accepted
 * @param rejected items the server rejected (validation, or a batch dropped on a 4xx)
 * @param failedBatches batches that failed and were put back for a retry
 */
public record ClientStats(
    long queued, long dropped, long sent, long rejected, long failedBatches) {}
