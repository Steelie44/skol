def fib(n)
  n < 2 ? n : fib(n - 1) + fib(n - 2)
end

t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
r = fib(30)
puts "RESULT #{r} #{Process.clock_gettime(Process::CLOCK_MONOTONIC) - t}"
