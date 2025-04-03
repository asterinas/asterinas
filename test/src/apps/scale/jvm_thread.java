import java.util.Random;

public class jvm_thread {
    public static void main(String[] args) throws InterruptedException {
        int threads_num = 1;
        if (args.length > 0) {
            threads_num = Integer.parseInt(args[0]);
        }

        System.out.println("***TEST_START***: ");
        System.out.println("Using " + threads_num + " threads");

        Thread[] threads = new Thread[threads_num];
        long[] finish_duration = new long[threads_num];

        long start_time = System.nanoTime();

        for (int t = 0; t < threads_num; t++) {
            int thread_id = t;
            threads[t] = new Thread(() -> {
                Random rng = new Random(19260817);
                recursion(500, rng);
                long finish_time = System.nanoTime();
                finish_duration[thread_id] = finish_time - start_time;
            });
        }

        for (int t = 0; t < threads_num; t++) {
            threads[t].start();
        }

        for (Thread thread : threads) {
            thread.join();
        }

        long lat = 0;
        for (int t = 0; t < threads_num; t++) {
            lat = Math.max(lat, finish_duration[t]);
        }
        System.out.println("Time consumed: " + lat + " ns");
        System.out.println("***TEST_END***: ");
    }

    public static int recursion(int n, Random rng) {
        if (n <= 0 || n % 2 == 0) return n + rng.nextInt(100);
        int r = rng.nextInt(100);
        int next1 = recursion(n - 1, rng);
        int next2 = recursion(n - 2, rng);
        int res = r + next1 - next2 + rng.nextInt(100);
        return res;
    }
}


