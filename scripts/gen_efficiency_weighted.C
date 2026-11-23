// Dev-only fixture generator: TEfficiency objects filled with weights, and
// the interval bounds ROOT gives each bin (GetEfficiency -/+ the errors).
//
//   root -l -b -q 'scripts/gen_efficiency_weighted.C'     # from the repo root
//
// Writes fixtures/efficiency_weighted.root and prints each bin's bounds.

void fill(TEfficiency &e, bool weighted, double w1) {
   // Bin 1: 6 passed of 10; bin 2: 1 of 4; bin 3: 5 of 5; bin 4: none.
   int passed[3] = {6, 1, 5}, total[3] = {10, 4, 5};
   for (int b = 0; b < 3; ++b)
      for (int i = 0; i < total[b]; ++i) {
         double w = weighted ? w1 + 0.25 * (i % 4) : 1.0;
         if (weighted)
            e.FillWeighted(i < passed[b], w, b + 0.5);
         else
            e.Fill(i < passed[b], b + 0.5);
      }
}

void gen_efficiency_weighted() {
   TFile f("fixtures/efficiency_weighted.root", "RECREATE");
   struct Case { const char *name; bool weighted; double w1; TEfficiency::EStatOption opt; bool shortest; double setw; };
   Case cases[] = {
      {"cp_weighted", true, 0.5, TEfficiency::kFCP, false, 1},
      {"normal_weighted", true, 0.5, TEfficiency::kFNormal, false, 1},
      {"jeffrey_weighted", true, 0.5, TEfficiency::kBJeffrey, false, 1},
      {"uniform_weighted", true, 0.5, TEfficiency::kBUniform, false, 1},
      {"wilson_weight_one", true, 1.0, TEfficiency::kFWilson, false, 1},
      {"cp_setweight", false, 1.0, TEfficiency::kFCP, false, 2.0},
      {"jeffrey_shortest", false, 1.0, TEfficiency::kBJeffrey, true, 1},
   };
   for (auto &c : cases) {
      TEfficiency e(c.name, c.name, 4, 0., 4.);
      e.SetStatisticOption(c.opt);
      e.SetConfidenceLevel(0.683);
      if (c.shortest) e.SetShortestInterval();
      if (c.setw != 1) e.SetWeight(c.setw);
      fill(e, c.weighted, c.w1);
      printf("%s bits weights=%d bayes=%d shortest=%d:", c.name, e.TestBit(BIT(18)),
             e.TestBit(BIT(14)), e.TestBit(BIT(16)));
      for (int b = 1; b <= 4; ++b) {
         double eff = e.GetEfficiency(b);
         printf(" (%.12g, %.12g)", eff - e.GetEfficiencyErrorLow(b), eff + e.GetEfficiencyErrorUp(b));
      }
      printf("\n");
      e.Write();
   }
}
