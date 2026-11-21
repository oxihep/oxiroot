// A TList of two TGraphs, each with a function in fFunctions. ROOT names
// TGraph, TList and TF1 in full once and by class back-references afterwards,
// so the second member refers to classes named inside the first.
//
//   root -l -b -q scripts/gen_list_functions.cpp
#include "TF1.h"
#include "TFile.h"
#include "TGraph.h"
#include "TList.h"

void gen_list_functions() {
    TFile f("fixtures/list_functions.root", "RECREATE", "", 0);
    double x[3] = {1, 2, 3};
    double ya[3] = {2, 4, 6};
    double yb[3] = {1, 4, 9};

    auto* a = new TGraph(3, x, ya);
    a->SetName("a");
    auto* fa = new TF1("lin", "[0]+[1]*x", 0, 4);
    fa->SetParameters(0.0, 2.0);
    a->GetListOfFunctions()->Add(fa);

    auto* b = new TGraph(3, x, yb);
    b->SetName("b");
    auto* fb = new TF1("sq", "[0]*x*x", 0, 4);
    fb->SetParameter(0, 1.0);
    b->GetListOfFunctions()->Add(fb);

    TList list;
    list.Add(a);
    list.Add(b);
    list.Write("graphs", TObject::kSingleKey);
    f.Close();
}
