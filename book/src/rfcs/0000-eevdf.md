# RFC-0000: An EEVDF scheduler for Asterinas

* Status: Draft
* Pull request: (link to PR)
* Date submitted: YYYY-MM-DD
* Date approved: YYYY-MM-DD

## Summary

This document proposes the adoption of the *Earliest Eligible Virtual Deadline First* (EEVDF) algorithm as Asterinas' fair scheduler.

## Motivation

Even though the current CFS implementation does a good job of fairly distributing CPU across processes, the algorithm wasn't originally designed to account for [latency-sensitive tasks](https://lwn.net/Articles/925371/).
Linux initially addressed these requirements with [latency-nice patches](https://lwn.net/Articles/887842/), but ultimately adopted the EEVDF algorithm as a more principled solution that balances latency and fairness constraints.

By adopting EEVDF, Asterinas can build on years of lessons learned from Linux and provide a robust, reliable execution environment for latency-sensitive workloads as well.

## Basic concepts

This section offers a brief introduction to a few key concepts involved in the EEVDF algorithm.

### Nice values and weights

Similar to Linux, Asterinas associates each task with a *nice value*, which represents its relative priority.
Nice values are mapped to scheduling *weights*, where a lower nice value corresponds to a higher weight.
Scheduling weights determine the proportion of CPU time allocated to a task relative to others.
For example, a task with twice the weight of another is expected to receive roughly twice as much CPU time over the long run.

### Virtual time

To reason about fairness and latency independently of wall-clock time, EEVDF uses the notion of *virtual time*.
Virtual time advances at a rate inversely proportional to a task’s weight: tasks with higher weights accumulate virtual runtime more slowly, reflecting the fact that they are entitled to a larger share of CPU.
This abstraction allows the scheduler to compare time-related quantities (such as deadlines) across tasks of different priorities.

### Virtual deadline

EEVDF assigns a fixed time slice for each task, which is converted to a virtual time slice.
Thus, tasks with higher weights end up with shorter virtual time slices.

The point in virtual time where the slice ends is the *virtual deadline*.

### Lag

*Lag* encapsulates the idea of how much CPU time the scheduler owes to a task.
To quantify the notion of lag, EEVDF subtracts the system's virtual runtime by the task's.
Since virtual time for tasks with higher weights advances slower, such tasks are more likely to acquire higher lag.

In the "Design" section we'll see how lag can influence deadlines and eligibility more precisely.

### Eligibility

Having the earliest virtual deadline is not the only criterion for a task to be picked next.
EEVDF picks the task with the earliest virtual deadline among the tasks that are *eligible*.

A task is eligible if its lag is non-negative.
That is, it's either even with the system's virtual runtime or it's owed CPU.

## Design

*This section will be no more rigorous than necessary.*

Linux's EEVDF reference implementation approximates the system's virtual runtime by the weighted average of the virtual runtime of all tasks.
To prevent overflows and loss of precision, however, this quantity is not stored directly.
Instead, for the set $S$ of tasks, the scheduler maintains

* The total weight of all tasks, $W = ∑_{i ∈ S}wᵢ$
* The minimum virtual runtime across all tasks, $\rho_{min}$
* The weighted sum of virtual runtime offsets, $\Phi = ∑_{i ∈ S}wᵢ(\rhoᵢ - \rho_{min})$

When necessary, the weighted average virtual runtime can be computed as

$$\bar{\rho} = \Phi / W + \rho_{min}$$

Part of the complexity comes from keeping those updated.
Especially $\Phi$, which will be explored in detail here.

### Enqueuing a task – `SchedClassRq::enqueue`

The first thing needed for an enqueued task $t$ is a stable choice of its virtual runtime $\rho_t$.

EEVDF places it at the average virtual runtime $\bar{\rho}$ subtracted by the task's virtual lag.
There's a caveat about the virtual lag needing an adjustment, but this detail can be discussed later.

Once $\rho_t$ is chosen, the virtual deadline for $t$ is defined as

$$\rho_t + qW_0 / w_t$$

Where $q$ is a fixed time slice, $W_0$ is a constant to mitigate loss of precision and $w_t$ is the weight of the task $t$.

Now that the deadline is defined, the task can be inserted in the eligibility queue (more on this later).

Finally, compute the updated $\Phi'$ as

$$
\begin{array}{rl}
\Phi' & = ∑_{i ∈ S\cup\{t\}}wᵢ(\rhoᵢ - \rho_{min}') & \\
  & = ∑_{i ∈ S}[wᵢ(\rhoᵢ - \rho_{min}')] + w_t(\rho_t - \rho_{min}') & \\
  & = ∑_{i ∈ S}[wᵢ(\rhoᵢ - \rho_{min})] - ∑_{i ∈ S}[wᵢ(\rho_{min}' - \rho_{min})] + w_t(\rho_t - \rho_{min}') & \\
  & = \Phi + W(\rho_{min} - \rho_{min}') + w_t(\rho_t - \rho_{min}') & \\
\end{array}
$$

Notably, there are two special cases:
* If the new minimum virtual runtime doesn't change, $\rho_{min} - \rho_{min}' = 0$
* If $t$ has the new minimum virtual runtime, $\rho_t - \rho_{min}' = 0$

### Choosing the next task – `SchedClassRq::pick_next`

Start by popping the next task from the eligibility queue.
Before setting it as the current task, some bookkeeping is required.

The task $t$ being rescheduled needs to have its virtual lag stored, computed as $\bar{\rho} - \rho_t$.
Here, Linux's EEVDF [clamps the virtual lag](https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L686) for stability.

Then, compute the updated $\Phi'$ as

$$
\begin{array}{rl}
\Phi' & = ∑_{i ∈ S\setminus\{t\}}[wᵢ(\rhoᵢ - \rho_{min}')] & \\
  & = ∑_{i ∈ S\setminus\{t\}}[wᵢ(\rhoᵢ - \rho_{min})] - ∑_{i ∈ S\setminus\{t\}}[wᵢ(\rho_{min}' - \rho_{min})] & \\
  & = \Phi - w_t(\rho_t - \rho_{min}) - W'(\rho_{min}' - \rho_{min}) & \\
\end{array}
$$

Again, there are two special cases:
* If the new minimum virtual runtime doesn't change, $\rho_{min} - \rho_{min}' = 0$
* If $t$ had the minimum virtual runtime, $\rho_t - \rho_{min} = 0$

### Updating the current task – `SchedClassRq::update_current`

When the current task $t$ is being updated after some wall-clock $\delta$, translate that to virtual time: $\Delta = \delta W_0 / w_t$.

The new virtual runtime for $t$ is updated as $\rho_t' = \rho_t + \Delta$.

Then, compute the updated $\Phi'$ as

$$
\begin{array}{rl}
\Phi' & = ∑_{i ∈ S}[wᵢ(\rhoᵢ' - \rho_{min}')] & \\
  & = ∑_{i ∈ S\setminus\{t\}}[wᵢ(\rhoᵢ - \rho_{min}')] + w_t(\rho_t + \Delta - \rho_{min}') & \\
  & = ∑_{i ∈ S\setminus\{t\}}[wᵢ(\rhoᵢ - \rho_{min} + \rho_{min} - \rho_{min}')] + w_t(\rho_t - \rho_{min}') + w_t\Delta & \\
  & = ∑_{i ∈ S\setminus\{t\}}[wᵢ(\rhoᵢ - \rho_{min})] + (\rho_{min} - \rho_{min}')∑_{i ∈ S\setminus\{t\}}[wᵢ] + w_t(\rho_t - \rho_{min}') + w_t\Delta & \\
  & = \Phi - w_t(\rho_t - \rho_{min}) + (\rho_{min} - \rho_{min}')(W - w_t) + w_t(\rho_t - \rho_{min}') + w_t\Delta & \\
  & = \Phi + w_t\Delta - W(\rho_{min}' - \rho_{min}) & \\
\end{array}
$$

If the minimum virtual runtime doesn't change, then $\rho_{min}' - \rho_{min} = 0$.

And finally, to know whether the current task needs to be rescheduled or not, the rules are:
1. If the eligibility queue is empty, then no. Otherwise,
2. If the current task is going to sleep or exiting, then yes. Otherwise,
3. If the current task is yielding or just ticking, return whether its allocated service has been fulfilled or not. That is, whether its virtual runtime has reached the virtual deadline.

### Adjusting the virtual lag when enqueuing a task

The [Linux source](https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L5230) explains this in detail.
Buf for the scope of this document, it's enough to reuse some of Linux's wording and state that to prevent virtual lag of a task $t$ of weight $w_t$ from quickly evaporating, it needs to be multiplied by $(W + w_t) / W$.

### The eligibility queue

The eligibility queue is, for the most part, a normal balanced tree whose nodes are keyed by the tasks' virtual deadlines.

However, while it's being traversed from top to bottom in search of the next task to pop, branches without eligible tasks can be pruned altogether with a smart trick.

From the virtual lag formula, a task $t$ is eligible when $\bar{\rho} - \rho_t \ge 0$.
Substituting $\bar{\rho}$:

$$
\begin{array}{c}
\Phi / W + \rho_{min} - \rho_t \ge 0 \\
(\rho_t - \rho_{min})W \le \Phi
\end{array}
$$

Then, each node is augmented with the minimum virtual runtime across its own task and the tasks of its children.
If the minimum virtual runtime of a branch is not small enough, then this branch doesn't have any eligible task.

So the traversal order is:
1. If the left child has an eligible task, find the task with the earliest deadline there. Otherwise,
2. If the task of the current node is eligible, choose it. Otherwise,
3. If the right child has an eligible task, find the task with the earliest deadline there. Otherwise,
4. Just return the leftmost task, ignoring eligibility.

The 4th rule is needed when the queue doesn't have any eligible task but the current task is going to sleep or exiting, forcing the algorithm to pick an ineligible task in order to avoid contention.

## Experiments

These are the results of some preliminary experiments.

Remarks:
* The tree used for the eligibility queue is an AVL tree, but an RB tree *may* be better
* These experiments used a base slice of 750000 ns, but Linux 6.16 has gone even lower than that, choosing [700000 ns](https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L79) as its minimal granularity.
* Of course, no magic is expected. There's a clear trade-off between throughput and latency when choosing the minimum granularity because of context switch overhead. What EEVDF offers is better latency control due to stronger, algebrically proven, guarantees.
* The "RPS percentiles" from `schbench` results were removed because those were just zeros.

```
===== SMP=1 =====

SMP=1/hackbench/CFS
1.521 1.524 1.526 1.527 1.528 1.528 1.528 1.538 1.544

SMP=1/hackbench/EEVDF
1.566 1.568 1.573 1.581 1.586 1.589 1.590 1.594 1.594

-----------------

SMP=1/schbench/CFS
Wakeup Latencies percentiles (usec) runtime 90 (s) (16442 total samples)
	  50.0th: 1          (0 samples)
	  90.0th: 1          (0 samples)
	* 99.0th: 1          (0 samples)
	  99.9th: 2          (15 samples)
	  min=1, max=17
Request Latencies percentiles (usec) runtime 90 (s) (26133 total samples)
	  50.0th: 3036       (9463 samples)
	  90.0th: 4028       (8262 samples)
	* 99.0th: 5032       (2346 samples)
	  99.9th: 5528       (233 samples)
	  min=2959, max=6347
average rps: 308.32

SMP=1/schbench/EEVDF
Wakeup Latencies percentiles (usec) runtime 90 (s) (21312 total samples)
	  50.0th: 1          (0 samples)
	  90.0th: 1          (0 samples)
	* 99.0th: 3          (525 samples)
	  99.9th: 5          (88 samples)
	  min=1, max=120
Request Latencies percentiles (usec) runtime 90 (s) (25689 total samples)
	  50.0th: 3036       (6022 samples)
	  90.0th: 4052       (9339 samples)
	* 99.0th: 5016       (2292 samples)
	  99.9th: 5560       (230 samples)
	  min=2968, max=6613
average rps: 303.50
```
```
===== SMP=8 =====

SMP=8/hackbench/CFS
0.484 0.490 0.506 0.555 0.583 0.597 0.603 0.605 0.682

SMP=8/hackbench/EEVDF
0.482 0.497 0.528 0.543 0.552 0.584 0.586 0.601 0.667

-----------------

SMP=8/schbench/CFS
Wakeup Latencies percentiles (usec) runtime 90 (s) (24283 total samples)
	  50.0th: 18         (7469 samples)
	  90.0th: 3132       (9377 samples)
	* 99.0th: 9680       (2198 samples)
	  99.9th: 10000      (211 samples)
	  min=1, max=16317
Request Latencies percentiles (usec) runtime 90 (s) (24291 total samples)
	  50.0th: 12912      (7243 samples)
	  90.0th: 69248      (9691 samples)
	* 99.0th: 186624     (2177 samples)
	  99.9th: 321024     (218 samples)
	  min=4229, max=492846
average rps: 286.62

SMP=8/schbench/EEVDF
Wakeup Latencies percentiles (usec) runtime 90 (s) (23137 total samples)
	  50.0th: 29         (6387 samples)
	  90.0th: 3404       (9250 samples)
	* 99.0th: 3964       (2064 samples)
	  99.9th: 4136       (204 samples)
	  min=1, max=4248
Request Latencies percentiles (usec) runtime 90 (s) (23145 total samples)
	  50.0th: 13072      (6797 samples)
	  90.0th: 68736      (9242 samples)
	* 99.0th: 223488     (2080 samples)
	  99.9th: 439808     (207 samples)
	  min=5462, max=633774
average rps: 273.10

-----------------

SMP=8/sysbench/cpu_lat/CFS
CPU speed:
    events per second:  2972.72

General statistics:
    total time:                          60.0013s
    total number of events:              178369

Latency (ms):
         min:                                    1.31
         avg:                                    1.35
         max:                                    2.41
         95th percentile:                        1.37
         sum:                               239969.09

Threads fairness:
    events (avg/stddev):           44592.2500/20.47
    execution time (avg/stddev):   59.9923/0.00

SMP=8/sysbench/cpu_lat/EEVDF
CPU speed:
    events per second:  2951.09

General statistics:
    total time:                          60.0012s
    total number of events:              177071

Latency (ms):
         min:                                    1.30
         avg:                                    1.36
         max:                                    2.84
         95th percentile:                        1.39
         sum:                               239961.30

Threads fairness:
    events (avg/stddev):           44267.7500/45.98
    execution time (avg/stddev):   59.9903/0.00

-----------------

SMP=8/sysbench/thread_lat/CFS
General statistics:
    total time:                          60.0045s
    total number of events:              4819117

Latency (ms):
         min:                                    0.01
         avg:                                    2.49
         max:                                   31.66
         95th percentile:                       11.87
         sum:                             11999580.94

Threads fairness:
    events (avg/stddev):           24095.5850/2326.61
    execution time (avg/stddev):   59.9979/0.00

SMP=8/sysbench/thread_lat/EEVDF
General statistics:
    total time:                          60.0049s
    total number of events:              4930920

Latency (ms):
         min:                                    0.01
         avg:                                    2.43
         max:                                   66.84
         95th percentile:                       11.65
         sum:                             11999397.10

Threads fairness:
    events (avg/stddev):           24654.6000/2000.30
    execution time (avg/stddev):   59.9970/0.00
```

## Drawbacks, alternatives, and unknown

- **Drawbacks:** a proper EEVDF scheduler implementation is visibly more complex than the current CFS. So code complexity is expected to increase.
- **Alternatives:** I (Arthur Paulino) honestly went straight into Linux's most modern solution due to its maturity, elegance and stability. I believe it otherwise would be difficult to balance fairness, latency-sensitivity and compatibility with Asterinas.
- **Unresolved Questions:** the weight update mechanism hasn't been covered, as it can be delivered as a separate patch. But to say it upfront, it requires extra care in order to keep the maintained quantities consistent. Especially $\Phi$!

## Implementation plan (if approved)

1. Adopt the implementation that's already done after a typical PR+review process, enhanced with extra documentation (probably extracted from this document)
2. Implement the weight update mechanism in a separate patch
3. Experiment an RB tree instead of an AVL tree as the underlying data structure for the eligibility queue. The performance difference is likely to be unnoticed by the current benchmarks, so maybe it makes sense to use sched clocks for extra precision.

## Prior art and references

* https://people.eecs.berkeley.edu/~istoica/papers/eevdf-tr-95.pdf
* https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c
* https://www.vittoriozaccaria.net/blog/notes-on-linux-eevdf
