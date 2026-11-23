// Writes fixtures/tf1_params.root: functions whose parameters ROOT numbers
// and names in ways a reader has to follow. Run from the repository root:
//   root -l -b -q scripts/gen_tf1_params.C
//
//   swap   [1]*x+[0]     stored as [p1]*x+[p0]: parameter 1 comes first
//   named  [b]*x+[a]     ROOT puts `a` first (its parameter order)
//   peak   gaus          parameters named Constant, Mean, Sigma
//   cb     crystalball   Constant, Mean, Sigma, Alpha, N, out of name order
//   lan    landau        Constant, MPV, Sigma, through TMath::Landau
//   bkg    expo(0)+pol1(2)
//
// Each is printed with its value at x = 1.3, for the test.
void gen_tf1_params() {
   TFile f("fixtures/tf1_params.root", "RECREATE");
   TF1 swap("swap", "[1]*x+[0]", 0, 5);
   swap.SetParameters(1.0, 1.5);
   TF1 named("named", "[b]*x+[a]", 0, 5);
   named.SetParameters(1.0, 1.5);
   TF1 peak("peak", "gaus", 0, 5);
   peak.SetParameters(10.0, 0.5, 2.0);
   TF1 cb("cb", "crystalball", -5, 5);
   cb.SetParameters(3.0, 1.0, 0.8, 1.2, 3.0);
   TF1 lan("lan", "landau", 0, 10);
   lan.SetParameters(4.0, 1.0, 0.7);
   TF1 bkg("bkg", "expo(0)+pol1(2)", 0, 5);
   bkg.SetParameters(0.5, -0.3, 2.0, 0.1);
   for (TF1 *fn : {&swap, &named, &peak, &cb, &lan, &bkg}) {
      fn->Write();
      printf("%s\t%.17g\t", fn->GetName(), fn->Eval(1.3));
      for (int i = 0; i < fn->GetNpar(); i++) printf("%s%s", i ? "," : "", fn->GetParName(i));
      printf("\n");
   }
}
