local function main()
  local t = os.clock()
  local acc = 0
  local n = 0
  while n < 10000000 do
    acc = acc + n * 2 - 1
    n = n + 1
  end
  print(string.format("RESULT %d %.6f", acc, os.clock() - t))
end
main()
