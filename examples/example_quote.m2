 F = (f = x -> toString \ flatten (
    if instance(x, List) then 
        if #x == 1 then 
            f(x#0) 
        else 
            for i 
            from 1 to (#x-1) 
            list f(x#i) 
    else 
        toString x
    )
 ) @@ parse
