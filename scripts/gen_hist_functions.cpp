// Histograms that carry functions in fFunctions, the way TH1::Fit leaves a
// fitted curve with its histogram: one under its own key with two functions
// (the second TF1 is named by a class back-reference), and two in a THStack.
//
//   root -l -b -q scripts/gen_hist_functions.cpp
#include "TF1.h"
#include "TFile.h"
#include "TH1F.h"
#include "THStack.h"
#include "TList.h"

static TH1F* fitted(const char* name, double scale) {
    auto* h = new TH1F(name, name, 4, 0, 4);
    h->SetDirectory(nullptr);
    h->Fill(0.5, scale);
    h->Fill(2.5, 2 * scale);
    auto* line = new TF1(Form("line_%s", name), "[0]+[1]*x", 0, 4);
    line->SetParameters(scale, 0.5);
    h->GetListOfFunctions()->Add(line);
    return h;
}

void gen_hist_functions() {
    TFile f("fixtures/hist_functions.root", "RECREATE", "", 0);

    auto* h = fitted("h", 1.0);
    auto* gaus = new TF1("peak", "gaus", 0, 4);
    gaus->SetParameters(3.0, 2.0, 0.5);
    h->GetListOfFunctions()->Add(gaus);
    h->Write();

    THStack stack("hs", "two fitted histograms");
    stack.Add(fitted("s1", 2.0));
    stack.Add(fitted("s2", 5.0));
    stack.Write();
    f.Close();
}
