local function main()
  local grid = {}
  for k = 0, 1023 do grid[k] = 0 end
  local names = {}
  for k = 0, 63 do names["e" .. k] = k end
  local t = os.clock()
  local acc = 0
  for rnd = 0, 1999 do
    for k = 0, 1023 do
      grid[k] = grid[k] + k
    end
    acc = acc + names["e" .. (rnd % 64)]
  end
  print(string.format("RESULT %d %.6f", acc + grid[1023], os.clock() - t))
end
main()
